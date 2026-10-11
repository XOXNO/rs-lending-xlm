//! Specula lending CR-20 / CR-21 second opinion, flipped by PR #196 (b965553).
//!
//! At HEAD one hub market whose `supplied x supply_index` (or
//! `borrowed x borrow_index`) product no longer fit `i128` made `accrue_step`
//! panic with `MathOverflow`, and because the pool's `get_bulk_indexes` view
//! runs the same step, every controller flow that values an account holding
//! that market (liquidation, bad-debt cleanup, force-socialization, withdraw,
//! borrow) reverted too. The fix caps each step's interest at the market value
//! ceiling (`MAX_MARKET_VALUE_RAY`): the market and every account holding it
//! stay live, cleanup needs no upgrade, and what the ceiling costs is further
//! interest, exactly like the 1e36 index ceiling. New supply into a capped
//! market is refused with `MathOverflow`.
//!
//! Fixture: USDC is the whale market (100,000 supplied, 97,000 borrowed, so
//! the default curve sits near its cap); twelve years of elapsed time take the
//! next accrual to the value ceiling. ETH is the collateral market and USDT
//! the debt market for the two small accounts under test. No market carries
//! seeded share-less cash: utilization reads supply shares, so a cash-only
//! book would run at the curve top once revenue shares exist. Amounts are
//! 7-decimal raw units.

use common::constants::{MAX_BORROW_INDEX_RAY, MAX_MARKET_VALUE_RAY};
use common::math::fp::Ray;
use common::types::PoolStateRaw;
use common::validation::max_cap_for_decimals;
use test_harness::{
    assert_contract_error, errors, hub_asset, usd, usdc_preset, usdt_stable_preset, LendingTest,
    MarketPreset, ALICE, BOB, CAROL, DAVE, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS,
    HARNESS_SPOKE, LIQUIDATOR,
};

const UNIT7: i128 = 10_000_000;
const YEAR_SECS: u64 = 31_556_926;
const USDC: &str = "USDC";
const USDT: &str = "USDT";
const ETH: &str = "ETH";

/// Whale market with no seeded cash, so the book is exactly BOB's supply.
fn whale_usdc() -> MarketPreset {
    let mut preset = usdc_preset();
    preset.initial_liquidity = 0.0;
    preset
}

/// Debt market for the small accounts; BOB supplies its only liquidity.
fn usdt_market() -> MarketPreset {
    let mut preset = usdt_stable_preset();
    preset.initial_liquidity = 0.0;
    preset
}

/// Collateral market; the suppliers' own ETH is its only cash.
fn eth_market() -> MarketPreset {
    MarketPreset {
        name: ETH,
        decimals: 7,
        price_wad: usd(2_000),
        initial_liquidity: 0.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// BOB supplies 100,000 USDC and 100,000 USDT; CAROL borrows 97,000 USDC
/// against ETH. ALICE holds one ETH, one raw unit of USDC (the dust leg in
/// the market that will hit the ceiling; a whole USDC would be worth a
/// million at the capped index and shield her) and 1,000 USDT of debt. DAVE
/// is the control: same ETH and USDT legs, no USDC.
fn fixture() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(whale_usdc())
        .with_market(eth_market())
        .with_market(usdt_market())
        .with_max_utilization_disabled_all_markets()
        .with_min_borrow_collateral_disabled()
        .build();
    lift_caps(&t, USDC, 7);

    t.supply_raw(BOB, USDC, 100_000 * UNIT7);
    t.supply_raw(BOB, USDT, 100_000 * UNIT7);
    t.supply_raw(CAROL, ETH, 200 * UNIT7);
    t.borrow_raw(CAROL, USDC, 97_000 * UNIT7);

    t.supply_raw(ALICE, ETH, UNIT7);
    t.supply_raw(ALICE, USDC, 1);
    t.borrow_raw(ALICE, USDT, 1_000 * UNIT7);

    t.supply_raw(DAVE, ETH, UNIT7);
    t.borrow_raw(DAVE, USDT, 1_000 * UNIT7);
    t
}

/// Lifts the spoke caps of `asset` to the token domain so the pool's own
/// market value ceiling, not the spoke cap floor-converted at a huge index,
/// is what refuses new supply.
fn lift_caps(t: &LendingTest, asset: &str, decimals: u32) {
    let cap = max_cap_for_decimals(decimals);
    let cfg = t.get_asset_config(asset);
    t.edit_asset_in_spoke_caps(
        asset,
        HARNESS_SPOKE,
        true,
        true,
        cfg.loan_to_value,
        cfg.liquidation_threshold,
        cfg.liquidation_bonus,
        cap,
        cap,
    );
}

fn pool_state(t: &LendingTest, asset: &str) -> PoolStateRaw {
    let key = hub_asset(t.resolve_asset(asset));
    t.pool_client(asset).get_sync_data(&key).state
}

/// `(debt value, supply value)` of a market in raw RAY at its stored indexes.
fn values(t: &LendingTest, asset: &str) -> (i128, i128) {
    let s = pool_state(t, asset);
    let env = &t.env;
    (
        Ray::from(s.borrowed)
            .mul(env, Ray::from(s.borrow_index))
            .raw(),
        Ray::from(s.supplied)
            .mul(env, Ray::from(s.supply_index))
            .raw(),
    )
}

#[test]
fn cr21_value_ceiling_keeps_every_valuation_and_cleanup_live() {
    let mut t = fixture();
    let alice = t.account_id(ALICE);
    let carol = t.account_id(CAROL);
    assert!(t.health_factor(ALICE) > 1.0);
    assert!(t.health_factor(DAVE) > 1.0);

    // Twelve years at ~142% APR on the whale market. At HEAD the next accrual
    // step overflowed the RAY value product (CR-20) and every valuation of an
    // account holding USDC reverted with MathOverflow (CR-21). Now the step
    // lands the book on the market value ceiling.
    t.advance_time(12 * YEAR_SECS);
    t.update_indexes_for(&[USDC]);
    let capped = pool_state(&t, USDC);
    let (debt_value, supply_value) = values(&t, USDC);
    assert!(
        capped.borrow_index < MAX_BORROW_INDEX_RAY,
        "the value ceiling binds before the index ceiling"
    );
    assert!(debt_value <= MAX_MARKET_VALUE_RAY && supply_value <= MAX_MARKET_VALUE_RAY);
    assert!(
        supply_value >= MAX_MARKET_VALUE_RAY / 1_000 * 999,
        "the supply side, which backs the debt plus the cash, sits on the ceiling: {supply_value}"
    );
    assert!(debt_value > MAX_MARKET_VALUE_RAY / 2);
    // Like the index ceiling, the value ceiling stops further interest.
    t.advance_time(YEAR_SECS);
    t.update_indexes_for(&[USDC, ETH, USDT]);
    let later = pool_state(&t, USDC);
    assert!(
        later.borrow_index - capped.borrow_index < capped.borrow_index / 1_000_000,
        "a capped market accrues nothing further"
    );
    // New supply into the capped market is refused with MathOverflow. The
    // revenue-share floor can leave the supply total up to one share (a
    // million USDC at this index) under the ceiling, so the probe deposits
    // one percent of the ceiling.
    assert_contract_error(
        t.try_supply(BOB, USDC, 1_000_000_000.0),
        errors::MATH_OVERFLOW,
    );

    // The controller values ALICE again; her risk-checked flows on healthy
    // legs work, and so does repayment.
    assert!(t.try_health_factor_raw(ALICE).is_some());
    t.try_withdraw_raw(ALICE, ETH, 1)
        .expect("withdraw of a healthy leg must not trap on the capped market");
    t.try_borrow(ALICE, USDT, 1.0)
        .expect("borrow of a healthy leg must not trap on the capped market");
    t.try_repay(ALICE, USDT, 100.0).expect("repay still works");

    // ETH halves: both small accounts become liquidatable on their ETH / USDT
    // legs, and ALICE's dust USDC leg no longer shields her.
    t.set_price(ETH, usd(1_000));
    assert!(t.health_factor(ALICE) < 1.0);
    assert!(t.health_factor(DAVE) < 1.0);
    t.get_or_create_user(LIQUIDATOR);
    assert_eq!(t.token_balance(LIQUIDATOR, ETH), 0.0);
    t.liquidate(LIQUIDATOR, ALICE, USDT, 500.0);
    assert!(
        t.token_balance(LIQUIDATOR, ETH) > 0.0,
        "liquidator must receive seized ETH from the account with the dust leg"
    );
    t.liquidate(LIQUIDATOR, DAVE, USDT, 500.0);
    // Cleanup on an account holding the capped market no longer traps: it
    // returns its own typed verdict.
    match t.try_clean_bad_debt_by_id(alice) {
        Ok(()) => {}
        Err(e) => assert_ne!(
            e,
            soroban_sdk::Error::from_contract_error(errors::MATH_OVERFLOW),
            "cleanup must not fail on the valuation"
        ),
    }

    // The whale borrower is hopelessly under water at the capped index; the
    // governance cleanup socializes the debt without a pool upgrade, and the
    // supplier can leave with the cash the market still holds.
    let before = pool_state(&t, USDC);
    t.try_force_socialize_bad_debt_by_id(carol)
        .expect("force socialization of the whale debt must succeed");
    let after = pool_state(&t, USDC);
    assert_eq!(after.borrowed, 0, "the whale debt is written off");
    assert!(
        after.supply_index < before.supply_index,
        "suppliers absorb it"
    );
    let bob_before = t.token_balance_raw(BOB, USDC);
    t.withdraw_all(BOB, USDC);
    let paid = t.token_balance_raw(BOB, USDC) - bob_before;
    assert!(
        paid > 0 && paid <= before.cash,
        "BOB exits with the cash: {paid}"
    );
}
