extern crate std;

use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, LendingTest, PositionType, ALICE,
};

// ---------------------------------------------------------------------------
// 1. test_withdraw_partial
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_partial() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.withdraw(ALICE, "USDC", 3_000.0);

    // Supply should be ~7000
    t.assert_supply_near(ALICE, "USDC", 7_000.0, 1.0);

    // Wallet should have received ~3000
    let wallet = t.token_balance(ALICE, "USDC");
    assert!(
        wallet > 2_999.0 && wallet < 3_001.0,
        "wallet should have ~3000 USDC, got {}",
        wallet
    );
}

// ---------------------------------------------------------------------------
// 2. test_withdraw_full_with_zero_amount
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_full_with_zero_amount() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.withdraw_all(ALICE, "USDC");

    // Supply balance should be 0
    let supply = t.supply_balance(ALICE, "USDC");
    assert!(
        supply < 0.01,
        "supply should be ~0 after withdraw_all, got {}",
        supply
    );

    // Wallet should have ~10k back
    let wallet = t.token_balance(ALICE, "USDC");
    assert!(
        wallet > 9_999.0,
        "wallet should have ~10k USDC, got {}",
        wallet
    );
}

// ---------------------------------------------------------------------------
// 3. test_withdraw_multiple_assets
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_multiple_assets() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 5.0);

    // Withdraw from both
    t.withdraw(ALICE, "USDC", 2_000.0);
    t.withdraw(ALICE, "ETH", 1.0);

    t.assert_supply_near(ALICE, "USDC", 8_000.0, 1.0);
    t.assert_supply_near(ALICE, "ETH", 4.0, 0.01);
}

// ---------------------------------------------------------------------------
// 4. test_withdraw_rejects_position_not_found
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_rejects_position_not_found() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);

    // Try to withdraw ETH -- ALICE has no ETH position
    let result = t.try_withdraw(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::POSITION_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// 5. test_withdraw_rejects_exceeding_hf
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_rejects_exceeding_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply $10k, borrow $3500 ETH (1.75 ETH) -- near LTV
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.75);

    // Withdrawing $6000 USDC would leave only $4k collateral
    // HF = (4000 * 0.80) / 3500 = 0.91 < 1.0 -- should fail
    let result = t.try_withdraw(ALICE, "USDC", 6_000.0);
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}

// ---------------------------------------------------------------------------
// 6. test_withdraw_allowed_without_borrows
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_allowed_without_borrows() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);

    // Full withdraw is OK when no borrows exist
    t.withdraw_all(ALICE, "USDC");

    let supply = t.supply_balance(ALICE, "USDC");
    assert!(supply < 0.01, "supply should be ~0");
}

// ---------------------------------------------------------------------------
// 7. test_withdraw_rejects_during_flash_loan
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_rejects_during_flash_loan() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}

// ---------------------------------------------------------------------------
// 8. test_withdraw_rejects_when_paused
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_rejects_when_paused() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.pause();

    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::CONTRACT_PAUSED);
}

// ---------------------------------------------------------------------------
// 9. test_withdraw_removes_position_when_empty
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_removes_position_when_empty() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);

    // Withdraw all USDC
    t.withdraw_all(ALICE, "USDC");

    // Should only have ETH supply
    t.assert_supply_count(ALICE, 1);
    t.assert_position_exists(ALICE, "ETH", PositionType::Supply);
}

// ---------------------------------------------------------------------------
// 10. test_withdraw_cleans_up_empty_account
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_cleans_up_empty_account() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.withdraw_all(ALICE, "USDC");

    // Account was auto-removed by cleanup_account_if_empty when all positions cleared
    let accounts = t.get_active_accounts(ALICE);
    assert_eq!(
        accounts.len(),
        0,
        "account should be auto-removed when empty"
    );
}

// ---------------------------------------------------------------------------
// 11. test_withdraw_full_amount_returned
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_full_amount_returned() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);

    // Wallet is 0 after supply
    let wallet_before = t.token_balance(ALICE, "USDC");
    assert!(wallet_before < 0.01);

    t.withdraw_all(ALICE, "USDC");

    let wallet_after = t.token_balance(ALICE, "USDC");
    assert!(
        wallet_after > 9_999.0,
        "wallet should have ~10k back, got {}",
        wallet_after
    );
}

// ---------------------------------------------------------------------------
// 12. test_withdraw_raw_precision
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_raw_precision() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Supply 1000 USDC raw units
    let supply_amount = 1000i128;
    t.supply_raw(ALICE, "USDC", supply_amount);

    // Withdraw 500 raw units
    t.withdraw_raw(ALICE, "USDC", 500i128);

    let remaining = t.supply_balance_raw(ALICE, "USDC");
    // Should be approximately 500 (may differ slightly due to index)
    assert!(
        (499..=501).contains(&remaining),
        "remaining supply should be ~500, got {}",
        remaining
    );
}
