//! Second opinion on Specula lending MC-4 and MC-5, driven through the public
//! `liquidate` entry point.
//!
//! MC-4: after governance lowers `max_supply_positions` below an account's
//! collateral count, a `Credit(0)` receiver has no room for the pro-rata
//! seizure (`require_credit_position_limit`), and `Transfer` mode needs cash
//! in every collateral market (`pool::ops::withdraw::gate_and_debit`). The
//! probe pins both reverts and then shows the path Specula did not consider:
//! the liquidator supplies the cash-less asset first, which `Transfer` mode
//! then pays out of.
//!
//! MC-5: a solvent, unhealthy account whose only collateral is a 0-decimal
//! leg cannot be liquidated through its unpaused debt leg when that leg,
//! grown by the bonus, backs less than one whole unit: the seizure floors to
//! zero, `release_unbacked_repayment` empties the plan, and `liquidate`
//! reverts with `InvalidPayments`. The probe pins the revert, the solvent
//! cleanup refusal, the boundary where the unpaused leg backs one unit, and
//! the multi-leg offer that works once the pause lifts.

use common::types::SeizeMode;
use test_harness::presets::{AssetConfigPreset, MarketPreset, DEFAULT_ASSET_CONFIG};
use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usdc_preset, wbtc_preset, LendingTest, ALICE,
    BOB, DEFAULT_MARKET_PARAMS, LIQUIDATOR,
};

// ---------------------------------------------------------------------------
// MC-4: position-limit cut plus a cash-less collateral market
// ---------------------------------------------------------------------------

/// USDC with no seeded cash, so the only USDC in the pool is what accounts
/// supply; max utilization disabled so a borrower can drain it to the 2%
/// liquidation buffer.
fn mc4_fixture() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(MarketPreset {
            initial_liquidity: 0.0,
            ..usdc_preset()
        })
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_max_utilization_disabled_all_markets()
        .build();

    // ALICE: two collateral legs, one debt leg. C = 16_000, D = 10_000.
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "WBTC", 0.1);
    t.borrow(ALICE, "ETH", 5.0);

    // BOB drains USDC cash down to the liquidation buffer (2% of 10_000).
    t.supply(BOB, "ETH", 20.0);
    t.borrow(BOB, "USDC", 9_700.0);
    assert!(
        t.pool_reserves("USDC") < 400.0,
        "USDC cash must be near the buffer, got {}",
        t.pool_reserves("USDC")
    );

    // ETH at 2_600: D = 13_000 against LT-weighted collateral 12_800.
    t.set_price("ETH", usd(2_600));
    assert!(t.can_be_liquidated(ALICE));
    t
}

/// MC-4 as stated: with the supply limit cut to 1 below ALICE's two collateral
/// legs, `Transfer` reverts on the cash-less USDC market and `Credit(0)` on the
/// receiver's position limit.
#[test]
fn mc4_limit_cut_and_cashless_market_block_both_seize_modes() {
    let mut t = mc4_fixture();
    t.set_position_limits(1, 1);

    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::INSUFFICIENT_LIQUIDITY,
    );
    assert_contract_error(
        t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0)),
        errors::POSITION_LIMIT_EXCEEDED,
    );
    assert!(t.can_be_liquidated(ALICE), "the account stays unhealthy");
}

/// Correction to MC-4's "no liquidation path": the liquidator supplies the
/// cash-less asset into a fresh one-slot account, which the lowered limit
/// permits, and `Transfer` mode then pays the seizure out of that cash. An
/// existing receiver still has no room for the second seized leg.
#[test]
fn mc4_liquidator_can_supply_the_cashless_asset_and_seize_in_transfer_mode() {
    let mut t = mc4_fixture();
    t.set_position_limits(1, 1);

    t.supply(LIQUIDATOR, "USDC", 3_000.0);
    let receiver = t.account_id(LIQUIDATOR);
    assert_contract_error(
        t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(receiver)),
        errors::POSITION_LIMIT_EXCEEDED,
    );

    let usdc_before = t.token_balance_raw(LIQUIDATOR, "USDC");
    let wbtc_before = t.token_balance_raw(LIQUIDATOR, "WBTC");
    t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0)
        .expect("Transfer mode pays from the cash the liquidator supplied");
    assert!(
        t.token_balance_raw(LIQUIDATOR, "USDC") > usdc_before,
        "the liquidator receives seized USDC"
    );
    assert!(
        t.token_balance_raw(LIQUIDATOR, "WBTC") > wbtc_before,
        "the liquidator receives seized WBTC"
    );
    assert!(
        t.borrow_balance(ALICE, "ETH") < 5.0,
        "the repayment retired ETH debt"
    );
}

/// Without the limit cut, `Credit(0)` is the documented answer to a cash-less
/// collateral market (ADR-0019): the fresh receiver takes both legs as shares.
#[test]
fn mc4_credit_zero_works_while_the_limit_still_fits_the_victims_legs() {
    let mut t = mc4_fixture();
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::INSUFFICIENT_LIQUIDITY,
    );
    let receiver = t
        .try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0))
        .expect("Credit(0) needs no collateral cash");
    assert!(t.supply_balance_raw_for(receiver, "USDC") > 0);
    assert!(t.supply_balance_raw_for(receiver, "WBTC") > 0);
}

// ---------------------------------------------------------------------------
// MC-5: 0-decimal collateral plus a paused debt leg
// ---------------------------------------------------------------------------

const GEM: &str = "GEM";

/// Collateral-only 0-decimal listing at $1_000 per unit; admission requires
/// `liquidation_fees == 0` and `can_borrow == false` below 3 decimals.
fn gem_preset() -> MarketPreset {
    MarketPreset {
        name: GEM,
        decimals: 0,
        price_wad: usd(1_000),
        initial_liquidity: 0.0,
        config: AssetConfigPreset {
            is_collateralizable: true,
            is_borrowable: false,
            is_flashloanable: false,
            flashloan_fee: 0,
            liquidation_fees: 0,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// ALICE holds 2 GEM units (LTV 75%, LT 80%) and borrows `usdc` dollars of
/// USDC and `eth` ETH, then GEM drops to $800: C = 1_600 against D = 1_400,
/// HF = 0.914, solvent. Finally the USDC debt listing is paused.
fn mc5_fixture(usdc: f64, eth: f64) -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(gem_preset())
        .build();
    t.supply(ALICE, GEM, 2.0);
    t.borrow(ALICE, "USDC", usdc);
    t.borrow(ALICE, "ETH", eth);
    t.set_price(GEM, usd(800));
    assert!(t.can_be_liquidated(ALICE));
    assert!(
        t.total_collateral(ALICE) >= t.total_debt(ALICE),
        "the account must be solvent"
    );
    t.set_spoke_asset_paused("USDC", true);
    t
}

/// MC-5 as stated, with its precondition made explicit: the unpaused ETH leg
/// is $400, which times the capped bonus (about 14.3%) backs 0.57 of the $800
/// unit. Every offer reverts: the paused leg with `SpokeAssetPaused`, the open
/// leg with `InvalidPayments` whatever its size, and the solvent account is
/// refused by `clean_bad_debt`.
#[test]
fn mc5_paused_leg_blocks_liquidation_when_the_open_leg_backs_under_one_unit() {
    let mut t = mc5_fixture(1_000.0, 0.2);

    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "USDC", 1_000.0),
        errors::SPOKE_ASSET_PAUSED,
    );
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.2),
        errors::INVALID_PAYMENTS,
    );
    // Over-offering is capped at the leg's debt; it does not reach a unit either.
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::INVALID_PAYMENTS,
    );
    assert_contract_error(
        t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 0.2, SeizeMode::Credit(0)),
        errors::INVALID_PAYMENTS,
    );
    let alice = t.account_id(ALICE);
    assert_contract_error(
        t.try_clean_bad_debt_by_id(alice),
        errors::CANNOT_CLEAN_BAD_DEBT,
    );
    assert_eq!(t.supply_balance_raw(ALICE, GEM), 2, "nothing was seized");
}

/// The block is the pause, not the whole-unit rule: once USDC reopens, a
/// multi-leg offer covering the whole debt closes the account.
#[test]
fn mc5_multi_leg_offer_succeeds_once_the_pause_lifts() {
    let mut t = mc5_fixture(1_000.0, 0.2);
    t.set_spoke_asset_paused("USDC", false);

    t.liquidate_multi(LIQUIDATOR, ALICE, &[("USDC", 1_000.0), ("ETH", 0.2)]);
    assert_eq!(t.borrow_balance_raw(ALICE, "USDC"), 0);
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), 0);
    assert!(t.token_balance_raw(LIQUIDATOR, GEM) >= 1);
}

/// Boundary: with the open ETH leg at $900, `900 * (1 + b)` backs one unit, so
/// the paused USDC leg does not block the liquidation. The seizure floors to
/// one unit and the repayment is trimmed to what that unit backs (about $700),
/// so the liquidator keeps part of its 0.45 ETH.
#[test]
fn mc5_open_leg_backing_one_unit_liquidates_despite_the_paused_leg() {
    let mut t = mc5_fixture(500.0, 0.45);

    t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.45)
        .expect("one whole unit is backed by the open leg");
    assert_eq!(t.token_balance_raw(LIQUIDATOR, GEM), 1, "one unit seized");
    assert_eq!(t.supply_balance_raw(ALICE, GEM), 1, "one unit left");
    let eth_debt = t.borrow_balance(ALICE, "ETH");
    assert!(
        eth_debt > 0.0 && eth_debt < 0.45,
        "the repayment was trimmed to the unit it backs, debt left {eth_debt}"
    );
    assert_eq!(
        t.borrow_balance(ALICE, "USDC"),
        500.0,
        "the paused leg is untouched"
    );
}
