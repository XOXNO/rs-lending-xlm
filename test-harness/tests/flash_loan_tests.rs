extern crate std;

use test_harness::{
    assert_contract_error, days, errors, eth_preset, usdc_preset, LendingTest, ALICE, BOB,
};

// ---------------------------------------------------------------------------
// 1. test_flash_loan_success_under_non_root_auth
// ---------------------------------------------------------------------------
// Under `mock_all_auths_allowing_non_root_auth()` (the harness default for
// contract-address auth chains), the nested `StellarAssetClient::mint()` call
// inside the good flash-loan receiver authorizes correctly and the flow
// completes end-to-end.

#[test]
fn test_flash_loan_success_under_non_root_auth() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply liquidity so the pool has funds.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Advance and sync to generate baseline revenue.
    t.advance_and_sync(days(30));

    let receiver = t.deploy_flash_loan_receiver();
    let result = t.try_flash_loan(BOB, "USDC", 10_000.0, &receiver);

    assert!(
        result.is_ok(),
        "flash loan with good receiver must succeed under non-root auth mock: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// 2. test_flash_loan_rejects_bad_repayment
// ---------------------------------------------------------------------------

#[test]
fn test_flash_loan_rejects_bad_repayment() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let bad_receiver = t.deploy_bad_flash_loan_receiver();
    let result = t.try_flash_loan(BOB, "USDC", 10_000.0, &bad_receiver);
    // The bad receiver triggers a cross-contract failure that surfaces as
    // a host error, not a specific contract error code.
    assert!(
        result.is_err(),
        "flash loan should fail when receiver doesn't repay"
    );
}

// ---------------------------------------------------------------------------
// 3. test_flash_loan_rejects_disabled
// ---------------------------------------------------------------------------

#[test]
fn test_flash_loan_rejects_disabled() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    // Disable flash loans for USDC.
    t.edit_asset_config("USDC", |cfg| {
        cfg.is_flashloanable = false;
    });

    let receiver = t.deploy_flash_loan_receiver();
    let result = t.try_flash_loan(BOB, "USDC", 10_000.0, &receiver);
    assert_contract_error(result, errors::FLASHLOAN_NOT_ENABLED);
}

// ---------------------------------------------------------------------------
// 4. test_flash_loan_rejects_zero_amount
// ---------------------------------------------------------------------------

#[test]
fn test_flash_loan_rejects_zero_amount() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let receiver = t.deploy_flash_loan_receiver();
    let result = t.try_flash_loan(BOB, "USDC", 0.0, &receiver);
    // Must reject with the precise AMOUNT_MUST_BE_POSITIVE (14).
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

// ---------------------------------------------------------------------------
// 5. test_flash_loan_reentrancy_blocks_supply
// ---------------------------------------------------------------------------

#[test]
fn test_flash_loan_reentrancy_blocks_supply() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_supply(BOB, "USDC", 1_000.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}

// ---------------------------------------------------------------------------
// 6. test_flash_loan_reentrancy_blocks_borrow
// ---------------------------------------------------------------------------

#[test]
fn test_flash_loan_reentrancy_blocks_borrow() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}

// ---------------------------------------------------------------------------
// 7. test_flash_loan_reentrancy_blocks_withdraw
// ---------------------------------------------------------------------------

#[test]
fn test_flash_loan_reentrancy_blocks_withdraw() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}

// ---------------------------------------------------------------------------
// 8. test_flash_loan_reentrancy_blocks_repay
// ---------------------------------------------------------------------------

#[test]
fn test_flash_loan_reentrancy_blocks_repay() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_repay(ALICE, "ETH", 0.5);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}

// ---------------------------------------------------------------------------
// 9. test_flash_loan_reentrancy_blocks_liquidation
// ---------------------------------------------------------------------------

#[test]
fn test_flash_loan_reentrancy_blocks_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", test_harness::usd_cents(50));
    t.assert_liquidatable(ALICE);

    t.set_flash_loan_ongoing(true);

    let result = t.try_liquidate(BOB, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}

// ---------------------------------------------------------------------------
// 10. test_flash_loan_fee_config_matches_default_preset
// ---------------------------------------------------------------------------

#[test]
fn test_flash_loan_fee_config_matches_default_preset() {
    // Pin the default preset config values so any change to
    // `usdc_preset()` surfaces in CI. The end-to-end fee transfer runs in
    // the inline `test_flash_loan` in pool/src/lib.rs, which uses the admin
    // (covered by mock_all_auths) as receiver.
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let config = t.get_asset_config("USDC");
    assert_eq!(
        config.flashloan_fee_bps, 9,
        "USDC preset flash-loan fee must be 9 BPS (0.09%)"
    );
    assert!(
        config.is_flashloanable,
        "USDC preset must have is_flashloanable = true"
    );
}
