//! GH-07, strategy verbs. `i128::MAX` as a debt amount fails on liquidity
//! before any transfer; as a collateral amount it means "all".

use controller::types::PositionMode;
use test_harness::{
    assert_contract_error, build_aggregator_swap, errors, f64_to_i128, LendingTest, ALICE, BOB,
};

fn setup() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 1_000.0);
    t.fund_router("ETH", 1_000.0);
    t.fund_router("USDC", 1_000_000.0);
    t
}

/// `f64` cannot carry `i128::MAX`; the harness converts back with `as i128`,
/// which saturates to exactly `i128::MAX`.
const MAX_AS_F64: f64 = i128::MAX as f64 / 10_000_000.0;

#[test]
fn multiply_with_i128_max_debt_fails_on_liquidity() {
    let mut t = setup();
    let steps = t.mock_swap_steps("ETH", "USDC", 0);
    assert_contract_error(
        t.try_multiply(
            ALICE,
            "USDC",
            MAX_AS_F64,
            "ETH",
            PositionMode::Multiply,
            &steps,
        )
        .map(|_| ()),
        errors::INSUFFICIENT_LIQUIDITY,
    );
}

#[test]
fn swap_collateral_with_i128_max_amount_means_all() {
    let mut t = setup();
    t.supply(ALICE, "USDC", 10_000.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 0, f64_to_i128(1.0, 7));
    t.try_swap_collateral(ALICE, "USDC", MAX_AS_F64, "ETH", &steps)
        .expect("an oversized collateral amount resolves to the full position");
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), 0);
    assert_eq!(t.supply_balance_raw(ALICE, "ETH"), f64_to_i128(1.0, 7));
}

#[test]
fn repay_debt_with_collateral_with_i128_max_collateral_means_all_and_refunds_the_excess() {
    let mut t = setup();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 0, f64_to_i128(1.5, 7));
    t.try_repay_debt_with_collateral(ALICE, "USDC", MAX_AS_F64, "ETH", &steps, false)
        .expect("all collateral is withdrawn and swapped; the unspent debt asset is refunded");
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), 0);
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), 0);
    let refund = t.token_balance_raw(ALICE, "ETH");
    assert!(
        refund > f64_to_i128(1.4, 7) && refund < f64_to_i128(1.5, 7) + 1,
        "the unspent half ETH minus the ceil unit lands with the caller: {refund}"
    );
}
