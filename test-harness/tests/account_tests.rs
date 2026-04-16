extern crate std;

use test_harness::{
    eth_preset, usdc_preset, usdt_stable_preset, LendingTest, ALICE, BOB, STABLECOIN_EMODE,
};

// ---------------------------------------------------------------------------
// 1. test_create_normal_account
// ---------------------------------------------------------------------------

#[test]
fn test_create_normal_account() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    let account_id = t.create_account(ALICE);
    assert!(account_id > 0, "account_id should be non-zero");

    let attrs = t.get_account_attributes(ALICE);
    assert!(!attrs.is_isolated);
    assert_eq!(attrs.e_mode_category_id, 0);
    assert_eq!(attrs.mode, common::types::PositionMode::Normal);
}

// ---------------------------------------------------------------------------
// 2. test_create_emode_account
// ---------------------------------------------------------------------------

#[test]
fn test_create_emode_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    let account_id = t.create_emode_account(ALICE, 1);
    assert!(account_id > 0);

    let attrs = t.get_account_attributes(ALICE);
    assert_eq!(attrs.e_mode_category_id, 1);
    assert!(!attrs.is_isolated);
}

// ---------------------------------------------------------------------------
// 3. test_create_isolated_account
// ---------------------------------------------------------------------------

#[test]
fn test_create_isolated_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market_config("USDC", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = 1_000_000_000_000_000_000_000_000;
            // $1M WAD
        })
        .build();

    let account_id = t.create_isolated_account(ALICE, "USDC");
    assert!(account_id > 0);

    let attrs = t.get_account_attributes(ALICE);
    assert!(attrs.is_isolated);
    assert_eq!(attrs.e_mode_category_id, 0);
}

// ---------------------------------------------------------------------------
// 4. test_create_account_full_custom
// ---------------------------------------------------------------------------

#[test]
fn test_create_account_full_custom() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // mode=1 for Multiply
    let account_id = t.create_account_full(ALICE, 0, common::types::PositionMode::Multiply, false);
    assert!(account_id > 0);

    let attrs = t.get_account_attributes(ALICE);
    assert_eq!(attrs.mode, common::types::PositionMode::Multiply);
    assert!(!attrs.is_isolated);
    assert_eq!(attrs.e_mode_category_id, 0);
}

// ---------------------------------------------------------------------------
// 5. test_remove_empty_account
// ---------------------------------------------------------------------------

#[test]
fn test_remove_empty_account() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.create_account(ALICE);
    // Account exists -- now remove it
    t.remove_account(ALICE);

    // Active accounts should be empty
    let accounts = t.get_active_accounts(ALICE);
    assert_eq!(
        accounts.len(),
        0,
        "account list should be empty after removal"
    );
}

// ---------------------------------------------------------------------------
// 6. test_remove_rejects_with_positions
// ---------------------------------------------------------------------------

#[test]
fn test_remove_rejects_with_positions() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 1_000.0);

    let result = t.try_remove_account(ALICE);
    assert!(
        result.is_err(),
        "remove should fail when account has positions"
    );
}

// ---------------------------------------------------------------------------
// 7. test_multiple_accounts_per_user
// ---------------------------------------------------------------------------

#[test]
fn test_multiple_accounts_per_user() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let id1 = t.create_account(ALICE);
    let id2 = t.create_account_full(ALICE, 0, common::types::PositionMode::Normal, false);
    assert_ne!(id1, id2, "accounts should have different IDs");

    // Supply to each
    t.supply_to(ALICE, id1, "USDC", 1_000.0);
    t.supply_to(ALICE, id2, "ETH", 0.5);

    let bal1 = t.supply_balance_for(ALICE, id1, "USDC");
    let bal2 = t.supply_balance_for(ALICE, id2, "ETH");
    assert!(bal1 > 999.0, "account 1 should have ~1000 USDC supply");
    assert!(bal2 > 0.49, "account 2 should have ~0.5 ETH supply");

    let accounts = t.get_active_accounts(ALICE);
    assert!(accounts.len() >= 2, "should have at least 2 accounts");
}

// ---------------------------------------------------------------------------
// 8. test_account_auto_removed_after_full_repay_withdraw
// ---------------------------------------------------------------------------

#[test]
fn test_account_auto_removed_after_full_repay_withdraw() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Repay in full
    t.repay(ALICE, "ETH", 1.01);

    // Withdraw all -- this triggers auto-removal of the account
    t.withdraw_all(ALICE, "USDC");

    // Account was auto-removed by cleanup_account_if_empty
    let accounts = t.get_active_accounts(ALICE);
    assert_eq!(
        accounts.len(),
        0,
        "account should be auto-removed when all positions empty"
    );
}

// ---------------------------------------------------------------------------
// 9. test_get_active_accounts
// ---------------------------------------------------------------------------

#[test]
fn test_get_active_accounts() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Initially no accounts
    t.get_or_create_user(ALICE);
    let accounts_before = t.get_active_accounts(ALICE);
    assert_eq!(accounts_before.len(), 0);

    t.create_account(ALICE);
    let accounts_after = t.get_active_accounts(ALICE);
    assert_eq!(accounts_after.len(), 1);
}

// ---------------------------------------------------------------------------
// 10. test_account_owner_verified
// ---------------------------------------------------------------------------

#[test]
fn test_account_owner_verified() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);

    // BOB should not be able to withdraw from ALICE's account. `mock_all_auths`
    // bypasses signature checks, so this test calls the controller directly
    // and relies on ownership validation.
    let alice_account_id = t.resolve_account_id(ALICE);
    let bob_addr = t.get_or_create_user(BOB);
    let usdc_addr = t.resolve_asset("USDC");

    let ctrl = t.ctrl_client();
    let withdrawals = soroban_sdk::vec![&t.env, (usdc_addr, 10_000_000_000i128)];
    let result = ctrl.try_withdraw(&bob_addr, &alice_account_id, &withdrawals);
    assert!(
        result.is_err() || result.unwrap().is_err(),
        "BOB should not be able to withdraw from ALICE's account"
    );
}
