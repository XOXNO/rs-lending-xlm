extern crate std;

use common::constants::WAD;

use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, wbtc_preset, LendingTest, PositionType,
    ALICE, BOB,
};

// ---------------------------------------------------------------------------
// 1. test_repay_partial
// ---------------------------------------------------------------------------

#[test]
fn test_repay_partial() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 2.0);

    // Repay 1 ETH.
    t.repay(ALICE, "ETH", 1.0);

    let borrow = t.borrow_balance(ALICE, "ETH");
    assert!(
        borrow > 0.99 && borrow < 1.01,
        "borrow should be ~1 ETH after partial repay, got {}",
        borrow
    );
    t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
}

// ---------------------------------------------------------------------------
// 2. test_repay_full_clears_position
// ---------------------------------------------------------------------------

#[test]
fn test_repay_full_clears_position() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Repay slightly more to clear the position fully.
    t.repay(ALICE, "ETH", 1.01);

    let borrow = t.borrow_balance(ALICE, "ETH");
    assert!(
        borrow < 0.01,
        "borrow should be ~0 after full repay, got {}",
        borrow
    );

    // The borrow position must be removed.
    t.assert_borrow_count(ALICE, 0);
}

// ---------------------------------------------------------------------------
// 3. test_repay_overpayment_refunded
// ---------------------------------------------------------------------------

#[test]
fn test_repay_overpayment_refunded() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Record Alice's ETH wallet before repay. After borrow, Alice holds
    // ~1 ETH in her wallet.
    let wallet_before = t.token_balance(ALICE, "ETH");

    // Repay 2.0 ETH (~1 ETH overpayment). repay() mints `amount` to the
    // user, so the wallet holds wallet_before + 2.0 after the mint. Repay
    // then takes ~1 ETH, leaving wallet_before + 1.0.
    t.repay(ALICE, "ETH", 2.0);

    let wallet_after = t.token_balance(ALICE, "ETH");
    // The excess must have been refunded: wallet_after should be roughly
    // wallet_before + 1.0.
    assert!(
        wallet_after > wallet_before + 0.9,
        "overpayment should be refunded: before={}, after={}",
        wallet_before,
        wallet_after
    );

    // The borrow must be cleared.
    let borrow = t.borrow_balance(ALICE, "ETH");
    assert!(borrow < 0.01, "borrow should be ~0");
}

// ---------------------------------------------------------------------------
// 4. test_repay_by_third_party
// ---------------------------------------------------------------------------

#[test]
fn test_repay_by_third_party() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Bob repays Alice's debt using the controller directly.
    let alice_account_id = t.resolve_account_id(ALICE);
    let bob_addr = t.get_or_create_user(BOB);
    let eth_market = t.resolve_market("ETH");
    let eth_addr = eth_market.asset.clone();

    // Mint ETH to Bob so he can pay.
    let repay_amount = 1_0100000i128; // 1.01 ETH (7 decimals)
    eth_market.token_admin.mint(&bob_addr, &repay_amount);

    let ctrl = t.ctrl_client();
    let payments = soroban_sdk::vec![&t.env, (eth_addr, repay_amount)];
    ctrl.repay(&bob_addr, &alice_account_id, &payments);

    // Alice's borrow must be cleared.
    let borrow = t.borrow_balance(ALICE, "ETH");
    assert!(
        borrow < 0.01,
        "ALICE's borrow should be ~0 after BOB's repay, got {}",
        borrow
    );
}

// ---------------------------------------------------------------------------
// 5. test_repay_multiple_assets
// ---------------------------------------------------------------------------

#[test]
fn test_repay_multiple_assets() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "WBTC", 0.01);

    // Repay both via the controller's bulk call.
    let account_id = t.resolve_account_id(ALICE);
    let addr = t.users.get(ALICE).unwrap().address.clone();
    let eth_addr = t.resolve_asset("ETH");
    let wbtc_addr = t.resolve_asset("WBTC");

    let eth_repay = 1_0100000i128; // 1.01 ETH
    let wbtc_repay = 1_100_000i128; // 0.011 WBTC

    // Mint tokens for repayment.
    t.resolve_market("ETH").token_admin.mint(&addr, &eth_repay);
    t.resolve_market("WBTC")
        .token_admin
        .mint(&addr, &wbtc_repay);

    let ctrl = t.ctrl_client();
    let payments = soroban_sdk::vec![&t.env, (eth_addr, eth_repay), (wbtc_addr, wbtc_repay)];
    ctrl.repay(&addr, &account_id, &payments);

    let eth_borrow = t.borrow_balance(ALICE, "ETH");
    let wbtc_borrow = t.borrow_balance(ALICE, "WBTC");
    assert!(
        eth_borrow < 0.01,
        "ETH borrow should be cleared, got {}",
        eth_borrow
    );
    assert!(
        wbtc_borrow < 0.0001,
        "WBTC borrow should be cleared, got {}",
        wbtc_borrow
    );
}

// ---------------------------------------------------------------------------
// 6. test_repay_rejects_zero_amount
// ---------------------------------------------------------------------------

#[test]
fn test_repay_rejects_zero_amount() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let result = t.try_repay(ALICE, "ETH", 0.0);
    // Must reject with the precise AMOUNT_MUST_BE_POSITIVE (14), not just any
    // failure, so regressions in the validator chain surface loudly.
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

// ---------------------------------------------------------------------------
// 7. test_repay_rejects_position_not_found
// ---------------------------------------------------------------------------

#[test]
fn test_repay_rejects_position_not_found() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    // No ETH borrow.

    let result = t.try_repay(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::POSITION_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// 8. test_repay_rejects_during_flash_loan
// ---------------------------------------------------------------------------

#[test]
fn test_repay_rejects_during_flash_loan() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_repay(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}

// ---------------------------------------------------------------------------
// 9. test_repay_isolated_debt_decremented
// ---------------------------------------------------------------------------

#[test]
fn test_repay_isolated_debt_decremented() {
    let ceiling = 100_000 * WAD; // $100k WAD
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market_config("USDC", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = ceiling;
        })
        .with_market(eth_preset())
        .with_market_config("ETH", |cfg| {
            cfg.isolation_borrow_enabled = true;
        })
        .build();

    t.create_isolated_account(ALICE, "USDC");
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 0.5);

    let debt_before = t.get_isolated_debt("USDC");
    assert!(debt_before > 0, "isolated debt should be > 0 after borrow");

    t.repay(ALICE, "ETH", 0.5);

    let debt_after = t.get_isolated_debt("USDC");
    assert!(
        debt_after < debt_before,
        "isolated debt should decrease after repay: before={}, after={}",
        debt_before,
        debt_after
    );
}

// ---------------------------------------------------------------------------
// 10. test_repay_cleans_up_empty_account
// ---------------------------------------------------------------------------

#[test]
fn test_repay_cleans_up_empty_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Repay in full.
    t.repay(ALICE, "ETH", 1.01);

    // Withdraw all: triggers auto-removal.
    t.withdraw_all(ALICE, "USDC");

    // The account was auto-removed when all positions cleared.
    let accounts = t.get_active_accounts(ALICE);
    assert_eq!(
        accounts.len(),
        0,
        "account should be auto-removed when empty"
    );
}
