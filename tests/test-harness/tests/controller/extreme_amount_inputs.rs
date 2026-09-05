//! GH-07. Amounts at the domain ceiling and at `i128::MAX` on every controller
//! money path. Each row pins either a typed protocol error raised before any
//! token moves, or a documented "means all" semantic.

use common::types::{HubAssetKey, SeizeMode};
use common::validation::max_cap_for_decimals;
use soroban_sdk::{vec, Bytes, Vec};
use test_harness::{
    assert_contract_error, errors, hub_asset, map_try_ok_unit, map_try_ok_value, usd, LendingTest,
    MarketPreset, ALICE, BOB, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, HARNESS_SPOKE,
    LIQUIDATOR,
};

fn market(name: &'static str, decimals: u32, price_wad: i128) -> MarketPreset {
    MarketPreset {
        name,
        decimals,
        price_wad,
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

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

fn setup(decimals: u32) -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(market("A", decimals, usd(1)))
        .with_market(market("USDC", 7, usd(1)))
        .with_min_borrow_collateral_disabled()
        .with_max_utilization_disabled_all_markets()
        .build();
    lift_caps(&t, "A", decimals);
    t.supply(BOB, "USDC", 100_000.0);
    t
}

fn leg(t: &LendingTest, asset: &str, amount: i128) -> Vec<(HubAssetKey, i128)> {
    vec![&t.env, (hub_asset(t.resolve_asset(asset)), amount)]
}

#[test]
fn supply_at_the_domain_ceiling_succeeds_and_one_unit_more_is_rejected() {
    for decimals in [3u32, 7, 18] {
        let mut t = setup(decimals);
        let cap = max_cap_for_decimals(decimals);
        t.supply_raw(ALICE, "A", cap);
        assert_eq!(
            t.supply_balance_raw(ALICE, "A"),
            cap,
            "decimals {decimals}: the ceiling books exactly"
        );
        let alice = t.get_or_create_user(ALICE);
        t.resolve_market("A").token_admin.mint(&alice, &1);
        let id = t.account_id(ALICE);
        // A second unit lands on a book that is already at the ceiling.
        let result = t
            .ctrl_client()
            .try_supply(&alice, &id, &HARNESS_SPOKE, &leg(&t, "A", 1));
        // The file's own contract is "a typed protocol error before any token
        // moves", so pin the code and prove the book did not move.
        assert_contract_error(map_try_ok_value(result), errors::MATH_OVERFLOW);
        assert_eq!(
            t.supply_balance_raw(ALICE, "A"),
            cap,
            "decimals {decimals}: the rejected unit must not book"
        );
    }
}

#[test]
fn supply_of_half_i128_max_is_a_typed_overflow_not_a_host_trap() {
    let mut t = setup(7);
    let alice = t.get_or_create_user(ALICE);
    let huge = i128::MAX / 2;
    t.resolve_market("A").token_admin.mint(&alice, &huge);
    let result = t
        .ctrl_client()
        .try_supply(&alice, &0, &HARNESS_SPOKE, &leg(&t, "A", huge));
    assert_contract_error(map_try_ok_value(result), errors::MATH_OVERFLOW);
}

#[test]
fn borrow_of_i128_max_fails_on_liquidity_before_any_transfer() {
    let mut t = setup(7);
    t.supply(ALICE, "USDC", 10_000.0);
    let id = t.account_id(ALICE);
    let cash_before = t.pool_reserves("A");
    let alice = t.get_or_create_user(ALICE);
    let result = t
        .ctrl_client()
        .try_borrow(&alice, &id, &leg(&t, "A", i128::MAX), &None);
    assert_contract_error(map_try_ok_unit(result), errors::INSUFFICIENT_LIQUIDITY);
    assert_eq!(t.pool_reserves("A"), cash_before);
}

#[test]
fn withdraw_of_i128_max_is_the_withdraw_all_sentinel() {
    let mut t = setup(7);
    let deposit = 5_000 * 10_000_000i128;
    t.supply_raw(ALICE, "A", deposit);
    t.withdraw_raw(ALICE, "A", i128::MAX);
    assert_eq!(t.supply_balance_raw(ALICE, "A"), 0);
    assert_eq!(t.token_balance_raw(ALICE, "A"), deposit);
}

#[test]
fn repay_of_half_i128_max_closes_the_debt_and_refunds_the_rest() {
    let mut t = setup(7);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "A", 1_000.0);
    let unit = 10_000_000i128;
    let huge = i128::MAX / 2;
    t.repay_raw(ALICE, "A", huge);
    assert_eq!(t.borrow_balance_raw(ALICE, "A"), 0);
    let wallet = t.token_balance_raw(ALICE, "A");
    let debt_paid = huge + 1_000 * unit - wallet;
    assert!(
        debt_paid >= 1_000 * unit && debt_paid <= 1_000 * unit + 1,
        "only the debt left the wallet: {debt_paid}"
    );
}

#[test]
fn liquidation_payment_of_half_i128_max_pulls_only_the_close_amount() {
    let mut t = setup(7);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "A", 7_000.0);
    t.set_price("A", usd(2));
    t.assert_liquidatable(ALICE);
    let huge = i128::MAX / 2;
    let before = t.borrow_balance_raw(ALICE, "A");
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market("A").token_admin.mint(&liquidator, &huge);
    let liq_before = t.token_balance_raw(LIQUIDATOR, "A");
    let victim = t.account_id(ALICE);
    t.ctrl_client().liquidate(
        &liquidator,
        &victim,
        &leg(&t, "A", huge),
        &SeizeMode::Transfer,
    );
    let after = t.borrow_balance_raw(ALICE, "A");
    assert!(after < before, "debt fell");
    // Two-sided: a lower bound alone is satisfied by a call that seizes
    // collateral while pulling zero debt token.
    let spent = liq_before - t.token_balance_raw(LIQUIDATOR, "A");
    assert_eq!(
        spent,
        before - after,
        "only the repaid amount left the liquidator: spent {spent}, repaid {}",
        before - after
    );
}

#[test]
fn recapitalize_of_half_i128_max_on_a_healthy_market_applies_zero_and_refunds_all() {
    let mut t = setup(7);
    let payer = t.get_or_create_user(ALICE);
    let huge = i128::MAX / 2;
    t.resolve_market("A").token_admin.mint(&payer, &huge);
    let applied = t
        .ctrl_client()
        .recapitalize(&payer, &hub_asset(t.resolve_asset("A")), &huge);
    assert_eq!(applied, 0);
    assert_eq!(t.token_balance_raw(ALICE, "A"), huge);
}

#[test]
fn flash_loan_of_i128_max_fails_on_liquidity() {
    let mut t = setup(7);
    let receiver = t.deploy_flash_loan_receiver();
    assert_contract_error(
        t.try_flash_loan_with_data(ALICE, "A", i128::MAX, &receiver, &Bytes::new(&t.env)),
        errors::INSUFFICIENT_LIQUIDITY,
    );
}
