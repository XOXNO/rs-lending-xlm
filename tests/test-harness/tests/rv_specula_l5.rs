//! Specula lending CR-20 / CR-21 second opinion: one hub market whose
//! `supplied x supply_index` (or `borrowed x borrow_index`) product no longer
//! fits `i128` makes `accrue_step` panic with `MathOverflow`. The pool's
//! `get_bulk_indexes` view runs the same step, so every controller flow that
//! values an account holding that market (liquidation, bad-debt cleanup,
//! force-socialization, withdraw, borrow) reverts, even when the legs being
//! liquidated are in healthy markets. Repay loads the debt market only and
//! still works. An account without a leg in the frozen market is unaffected.
//!
//! Fixture: USDC is the whale market (100,000 supplied, 97,000 borrowed, so
//! the default curve sits near its cap); twelve years of elapsed time push the
//! next accrual past the RAY value ceiling. ETH is the collateral market and
//! USDT the debt market for the two small accounts under test. No market
//! carries seeded share-less cash: utilization reads supply shares, so a
//! cash-only book would run at the curve top once revenue shares exist.
//! Amounts are 7-decimal raw units.

use test_harness::{
    assert_contract_error, errors, usd, usdc_preset, usdt_stable_preset, LendingTest, MarketPreset,
    ALICE, BOB, CAROL, DAVE, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, LIQUIDATOR,
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
/// against ETH. ALICE holds one ETH, one USDC (the dust leg in the market
/// that will freeze) and 1,000 USDT of debt. DAVE is the control: same ETH
/// and USDT legs, no USDC.
fn fixture() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(whale_usdc())
        .with_market(eth_market())
        .with_market(usdt_market())
        .with_max_utilization_disabled_all_markets()
        .with_min_borrow_collateral_disabled()
        .build();

    t.supply_raw(BOB, USDC, 100_000 * UNIT7);
    t.supply_raw(BOB, USDT, 100_000 * UNIT7);
    t.supply_raw(CAROL, ETH, 200 * UNIT7);
    t.borrow_raw(CAROL, USDC, 97_000 * UNIT7);

    t.supply_raw(ALICE, ETH, UNIT7);
    t.supply_raw(ALICE, USDC, UNIT7);
    t.borrow_raw(ALICE, USDT, 1_000 * UNIT7);

    t.supply_raw(DAVE, ETH, UNIT7);
    t.borrow_raw(DAVE, USDT, 1_000 * UNIT7);
    t
}

#[test]
fn cr21_frozen_market_leg_blocks_liquidation_and_cleanup_of_healthy_legs() {
    let mut t = fixture();
    let alice = t.account_id(ALICE);
    assert!(t.health_factor(ALICE) > 1.0);
    assert!(t.health_factor(DAVE) > 1.0);

    // Twelve years at ~142% APR on the whale market: the next accrual step
    // overflows the RAY value product. ETH then halves so both small accounts
    // become liquidatable on their ETH / USDT legs.
    t.advance_time(12 * YEAR_SECS);
    t.set_price(ETH, usd(1_000));

    // CR-20 at HEAD: the whale market can no longer accrue.
    assert_contract_error(t.try_update_indexes_for(&[USDC]), errors::MATH_OVERFLOW);
    // The other two markets accrue normally.
    t.update_indexes_for(&[ETH, USDT]);

    // The controller cannot even value ALICE: the health view runs the same
    // bulk index projection.
    assert!(
        t.try_health_factor_raw(ALICE).is_none(),
        "health view must fail for an account holding the frozen market"
    );
    let dave_hf = t.health_factor(DAVE);
    assert!(
        dave_hf < 1.0,
        "control account must be liquidatable: {dave_hf}"
    );

    // CR-21: liquidation of ALICE's USDT debt against her ETH collateral
    // reverts with MathOverflow, not a liquidation error. Neither cleanup
    // path admits her either.
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, USDT, 500.0),
        errors::MATH_OVERFLOW,
    );
    assert_contract_error(t.try_clean_bad_debt_by_id(alice), errors::MATH_OVERFLOW);
    assert_contract_error(
        t.try_force_socialize_bad_debt_by_id(alice),
        errors::MATH_OVERFLOW,
    );

    // ALICE's own risk-checked exits on healthy legs are blocked too.
    assert_contract_error(t.try_withdraw_raw(ALICE, ETH, 1), errors::MATH_OVERFLOW);
    assert_contract_error(t.try_borrow(ALICE, USDT, 1.0), errors::MATH_OVERFLOW);
    // Repay loads the debt market only, so she can still reduce her debt.
    t.try_repay(ALICE, USDT, 100.0)
        .expect("repay of a healthy leg must not touch the frozen market");

    // Control: DAVE, with no leg in the frozen market, is liquidated as usual
    // and the liquidator is paid ETH from pool cash. (At HF 0.70 the account
    // sits below threshold x (1 + bonus) = 0.84, so a partial liquidation
    // lowers its HF; the receipt, not the HF direction, proves the seizure.)
    assert_eq!(t.token_balance(LIQUIDATOR, ETH), 0.0);
    t.liquidate(LIQUIDATOR, DAVE, USDT, 500.0);
    assert!(
        t.token_balance(LIQUIDATOR, ETH) > 0.0,
        "liquidator must receive seized ETH from the control account"
    );
    assert!(
        t.try_health_factor_raw(DAVE).is_some(),
        "control account stays valuable after liquidation"
    );
}
