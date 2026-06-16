use test_harness::{assert_contract_error, errors, eth_preset, usdc_preset, LendingTest, ALICE};

use crate::helpers::build_swap_steps;
// 1. test_multiply_rejects_non_borrowable_debt -- asserts ASSET_NOT_BORROWABLE

#[test]
fn test_multiply_rejects_non_borrowable_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("ETH", |c| {
            c.is_borrowable = false;
        })
        .build();

    // ETH is not borrowable: multiply must fail with ASSET_NOT_BORROWABLE,
    // not an upstream pause or flash-loan guard error.
    let steps = build_swap_steps(&t, "ETH", "USDC", 1_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::ASSET_NOT_BORROWABLE);
}
// 2. test_multiply_rejects_non_collateralizable -- asserts NOT_COLLATERAL

#[test]
fn test_multiply_rejects_non_collateralizable() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("USDC", |c| {
            c.is_collateralizable = false;
        })
        .build();

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::NOT_COLLATERAL);
}
// 3. test_multiply_rejects_during_flash_loan -- asserts FLASH_LOAN_ONGOING

#[test]
fn test_multiply_rejects_during_flash_loan() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Set the flash-loan ongoing flag to simulate reentrancy.
    t.set_flash_loan_ongoing(true);

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}

