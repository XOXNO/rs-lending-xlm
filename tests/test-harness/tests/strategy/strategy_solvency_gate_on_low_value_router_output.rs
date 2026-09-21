//! The post-pool solvency gate (`strategies/mod.rs:53`) is the last line between a
//! low-value router output and an under-collateralised position. `MockAggregator`
//! pays exactly `min_out`, so `min_out` IS the router output here.

use controller::types::PositionMode;
use soroban_sdk::token;
use test_harness::{
    apply_flash_fee, assert_contract_error, build_aggregator_swap, errors, LendingTest, ALICE,
};

const ONE_ETH: i128 = 10_000_000;

fn try_multiply_one_eth_into_usdc(
    t: &mut LendingTest,
    usdc_out: i128,
) -> Result<u64, soroban_sdk::Error> {
    let steps = build_aggregator_swap(t, "ETH", "USDC", apply_flash_fee(ONE_ETH), usdc_out);
    t.try_multiply(ALICE, "USDC", 1.0, "ETH", PositionMode::Multiply, &steps)
}

#[test]
fn multiply_with_router_output_below_the_ltv_limit_is_insufficient_collateral() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.fund_router("USDC", 3_000.0);
    let alice = t.get_or_create_user(ALICE);
    let usdc = token::Client::new(&t.env, &t.resolve_asset("USDC"));
    let eth = token::Client::new(&t.env, &t.resolve_asset("ETH"));
    let pool = t.resolve_market("ETH").pool.clone();
    let (router_usdc, pool_eth) = (usdc.balance(&t.aggregator), eth.balance(&pool));

    // 1 ETH debt = $2 000. $2 600 of USDC: LTV-weighted $1 950 < $2 000, yet the
    // health factor would be 2 600 * 0.8 / 2 000 = 1.04. Only the LTV gate stops it.
    let below_ltv = try_multiply_one_eth_into_usdc(&mut t, 2_600 * ONE_ETH);
    assert_contract_error(below_ltv, errors::INSUFFICIENT_COLLATERAL);

    // One USDC unit under the exact limit: 2 666.6666666 * 0.75 < 2 000.
    let one_unit_short = try_multiply_one_eth_into_usdc(&mut t, 26_666_666_666);
    assert_contract_error(one_unit_short, errors::INSUFFICIENT_COLLATERAL);

    assert!(
        !t.account_exists(1),
        "a reverted multiply must not leave an account"
    );
    assert_eq!(t.get_active_accounts(ALICE).len(), 0);
    assert_eq!(usdc.balance(&t.aggregator), router_usdc);
    assert_eq!(eth.balance(&pool), pool_eth);
    assert_eq!(eth.balance(&alice), 0);
    assert_eq!(usdc.balance(&alice), 0);

    // First amount at the limit: 2 666.6666667 * 0.75 >= 2 000.
    let account_id = try_multiply_one_eth_into_usdc(&mut t, 26_666_666_667)
        .expect("router output exactly covering the LTV limit must open the position");
    assert_eq!(t.borrow_balance_raw_for(account_id, "ETH"), ONE_ETH);
}

#[test]
fn swap_collateral_into_a_lower_value_amount_with_open_debt_is_insufficient_collateral() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 30.0); // $60 000 debt against a $75 000 limit
    t.fund_router("ETH", 20.0);
    let eth = token::Client::new(&t.env, &t.resolve_asset("ETH"));
    let router_eth = eth.balance(&t.aggregator);
    let usdc_supply = t.supply_balance_raw(ALICE, "USDC");
    let debt = t.borrow_balance_raw(ALICE, "ETH");

    // 50 000 USDC -> 13 ETH ($26 000): collateral $76 000, LTV-weighted $57 000 <
    // $60 000, health factor 60 800 / 60 000 = 1.013. Only the LTV gate stops it.
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 0, 13 * ONE_ETH);
    let below_ltv = t.try_swap_collateral(ALICE, "USDC", 50_000.0, "ETH", &steps);
    assert_contract_error(below_ltv, errors::INSUFFICIENT_COLLATERAL);

    // 15 ETH is the exact limit ($80 000 * 0.75 = $60 000); one unit less reverts.
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 0, 15 * ONE_ETH - 1);
    let one_unit_short = t.try_swap_collateral(ALICE, "USDC", 50_000.0, "ETH", &steps);
    assert_contract_error(one_unit_short, errors::INSUFFICIENT_COLLATERAL);

    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), usdc_supply);
    assert_eq!(t.supply_balance_raw(ALICE, "ETH"), 0);
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), debt);
    assert_eq!(eth.balance(&t.aggregator), router_eth);

    let steps = build_aggregator_swap(&t, "USDC", "ETH", 0, 15 * ONE_ETH);
    t.try_swap_collateral(ALICE, "USDC", 50_000.0, "ETH", &steps)
        .expect("output exactly covering the LTV limit must pass");
    assert_eq!(t.supply_balance_raw(ALICE, "ETH"), 15 * ONE_ETH);
}
