use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
use soroban_sdk::IntoVal;
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, map_try_ok_unit, HubAssetKey,
    LendingTest, PositionType, ALICE, BOB,
};
#[test]
fn test_repay_partial() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 2.0);

    t.repay(ALICE, "ETH", 1.0);

    let borrow = t.borrow_balance(ALICE, "ETH");
    assert!(
        borrow > 0.99 && borrow < 1.01,
        "borrow should be ~1 ETH after partial repay, got {}",
        borrow
    );
    t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
}
#[test]
fn test_repay_full_clears_position() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let wallet_before = t.token_balance(ALICE, "ETH");

    t.repay(ALICE, "ETH", 1.01);

    let wallet_after = t.token_balance(ALICE, "ETH");
    assert!(
        (wallet_after - wallet_before).abs() < 0.05,
        "wallet delta should be ~0 after exact-repay (auto-mint cancels transfer): before={}, after={}",
        wallet_before,
        wallet_after
    );

    let borrow = t.borrow_balance(ALICE, "ETH");
    assert!(
        borrow < 0.01,
        "borrow should be ~0 after full repay, got {}",
        borrow
    );

    t.assert_borrow_count(ALICE, 0);
}
#[test]
fn test_repay_overpayment_refunded() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let debt_before = t.borrow_balance_raw(ALICE, "ETH");
    let wallet_before = t.token_balance_raw(ALICE, "ETH");
    let cash_before = t.pool_reserves("ETH");
    let overpay = 2 * 10_000_000i128; // 2.0 ETH against a 1.0 ETH debt

    // `repay_raw` mints `overpay` to ALICE first, so the wallet delta is the refund.
    t.repay_raw(ALICE, "ETH", overpay);

    assert_eq!(
        t.token_balance_raw(ALICE, "ETH") - wallet_before,
        overpay - debt_before,
        "the refund must be the overpayment and nothing more"
    );
    assert_eq!(
        t.borrow_balance_raw(ALICE, "ETH"),
        0,
        "the debt must be fully cleared"
    );
    assert_eq!(
        t.pool_reserves("ETH") - cash_before,
        1.0,
        "the pool banks exactly the repaid principal; the overpayment is never credited to cash"
    );
    t.assert_borrow_count(ALICE, 0);
}
#[test]
fn test_repay_allowed_when_paused() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.pause();

    let result = t.try_repay(ALICE, "ETH", 0.5);
    assert!(result.is_ok(), "repay should remain available while paused");

    t.assert_borrow_near(ALICE, "ETH", 0.5, 0.01);
}

#[test]
fn test_repay_by_third_party() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let alice_account_id = t.resolve_account_id(ALICE);
    let bob_addr = t.get_or_create_user(BOB);
    let eth_market = t.resolve_market("ETH");
    let eth_addr = eth_market.asset.clone();

    let repay_amount = 1_0100000i128;
    eth_market.token_admin.mint(&bob_addr, &repay_amount);
    let bob_before = t.token_balance(BOB, "ETH");

    let ctrl = t.ctrl_client();
    let payments = soroban_sdk::vec![&t.env, (hub_asset(eth_addr), repay_amount)];
    ctrl.repay(&bob_addr, &alice_account_id, &payments);

    let borrow = t.borrow_balance(ALICE, "ETH");
    assert!(
        borrow < 0.01,
        "ALICE's borrow should be ~0 after BOB's repay, got {}",
        borrow
    );
    let bob_after = t.token_balance(BOB, "ETH");
    assert!(
        bob_before - bob_after >= 0.99,
        "Bob's wallet must be debited by ~1.0 ETH for Alice's repay: before={}, after={}",
        bob_before,
        bob_after
    );
    assert_eq!(
        t.token_balance(ALICE, "ETH"),
        1.0,
        "Alice's wallet must be untouched by Bob's repay"
    );
}

#[test]
fn test_repay_permissionless_payer_auth_only() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let alice_account_id = t.resolve_account_id(ALICE);
    let bob_addr = t.get_or_create_user(BOB);
    let eth_market = t.resolve_market("ETH");
    let eth_addr = eth_market.asset.clone();
    let pool_addr = eth_market.pool.clone();

    let repay_amount = 1_0100000i128;
    eth_market.token_admin.mint(&bob_addr, &repay_amount);
    let bob_before = t.token_balance(BOB, "ETH");

    let payments = soroban_sdk::vec![&t.env, (hub_asset(eth_addr.clone()), repay_amount)];

    let transfer_args = (bob_addr.clone(), pool_addr.clone(), repay_amount).into_val(&t.env);
    let transfer_invoke = MockAuthInvoke {
        contract: &eth_addr,
        fn_name: "transfer",
        args: transfer_args,
        sub_invokes: &[],
    };
    let repay_args = (bob_addr.clone(), alice_account_id, payments.clone()).into_val(&t.env);
    let repay_invoke = MockAuthInvoke {
        contract: &t.controller,
        fn_name: "repay",
        args: repay_args,
        sub_invokes: core::slice::from_ref(&transfer_invoke),
    };
    let auths = [MockAuth {
        address: &bob_addr,
        invoke: &repay_invoke,
    }];

    t.ctrl_client()
        .mock_auths(&auths)
        .repay(&bob_addr, &alice_account_id, &payments);

    assert!(
        t.borrow_balance(ALICE, "ETH") < 0.01,
        "Alice's debt must clear on a repay she never authorized"
    );
    assert_eq!(
        t.token_balance(ALICE, "ETH"),
        1.0,
        "Alice's wallet must be untouched"
    );

    let bob_after = t.token_balance(BOB, "ETH");
    let bob_paid = bob_before - bob_after;
    assert!(
        (0.99..1.005).contains(&bob_paid),
        "Bob must be debited ~1.0 ETH (debt), with the ~0.01 overpayment refunded to him, got {}",
        bob_paid
    );
}
#[test]
fn test_repay_multiple_assets() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "WBTC", 0.01);

    let account_id = t.resolve_account_id(ALICE);
    let addr = t.users.get(ALICE).unwrap().address.clone();
    let eth_addr = t.resolve_asset("ETH");
    let wbtc_addr = t.resolve_asset("WBTC");

    let eth_repay = 1_0100000i128;
    let wbtc_repay = 1_100_000i128;

    t.resolve_market("ETH").token_admin.mint(&addr, &eth_repay);
    t.resolve_market("WBTC")
        .token_admin
        .mint(&addr, &wbtc_repay);
    let eth_before = t.token_balance(ALICE, "ETH");
    let wbtc_before = t.token_balance(ALICE, "WBTC");

    let ctrl = t.ctrl_client();
    let payments = soroban_sdk::vec![
        &t.env,
        (hub_asset(eth_addr), eth_repay),
        (hub_asset(wbtc_addr), wbtc_repay)
    ];
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
    let eth_after = t.token_balance(ALICE, "ETH");
    let wbtc_after = t.token_balance(ALICE, "WBTC");
    assert!(
        eth_before - eth_after >= 0.99,
        "ETH wallet must drop by ~1.0 after repay: before={}, after={}",
        eth_before,
        eth_after
    );
    assert!(
        wbtc_before - wbtc_after >= 0.0099,
        "WBTC wallet must drop by ~0.01 after repay: before={}, after={}",
        wbtc_before,
        wbtc_after
    );
}
#[test]
fn test_repay_rejects_zero_amount() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let result = t.try_repay(ALICE, "ETH", 0.0);

    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

#[test]
fn test_repay_rejects_empty_payment_vector() {
    let mut t = LendingTest::new().with_market(eth_preset()).build();

    let caller = t.get_or_create_user(ALICE);
    let payments: soroban_sdk::Vec<(HubAssetKey, i128)> = soroban_sdk::vec![&t.env];
    let result = map_try_ok_unit(t.ctrl_client().try_repay(&caller, &999_999u64, &payments));

    assert_contract_error(result, errors::INVALID_PAYMENTS);
}

#[test]
fn test_repay_rejects_negative_raw_amount() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let caller = t.users.get(ALICE).unwrap().address.clone();
    let account_id = t.resolve_account_id(ALICE);
    let eth = t.resolve_asset("ETH");
    let payments = soroban_sdk::vec![&t.env, (hub_asset(eth), -1i128)];
    let result = map_try_ok_unit(t.ctrl_client().try_repay(&caller, &account_id, &payments));

    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

#[test]
fn test_repay_duplicate_asset_payments_aggregate() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let caller = t.users.get(ALICE).unwrap().address.clone();
    let account_id = t.resolve_account_id(ALICE);
    let eth_market = t.resolve_market("ETH");
    let eth = eth_market.asset.clone();
    eth_market.token_admin.mint(&caller, &1_0100000i128);

    let payments = soroban_sdk::vec![
        &t.env,
        (hub_asset(eth.clone()), 5000000i128),
        (hub_asset(eth), 5100000i128)
    ];
    t.ctrl_client().repay(&caller, &account_id, &payments);

    let borrow = t.borrow_balance(ALICE, "ETH");
    assert!(
        borrow < 0.01,
        "duplicate repayment entries should aggregate and clear the debt, got {}",
        borrow
    );
    t.assert_borrow_count(ALICE, 0);
}

#[test]
fn test_repay_rejects_nonexistent_account_id() {
    let mut t = LendingTest::new().with_market(eth_preset()).build();

    let caller = t.get_or_create_user(ALICE);
    let eth = t.resolve_asset("ETH");
    let payments = soroban_sdk::vec![&t.env, (hub_asset(eth), 1i128)];
    let result = map_try_ok_unit(t.ctrl_client().try_repay(&caller, &999_999u64, &payments));

    assert_contract_error(result, errors::ACCOUNT_NOT_IN_MARKET);
}
#[test]
fn test_repay_rejects_position_not_found() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);

    let result = t.try_repay(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::DEBT_POSITION_NOT_FOUND);
}
#[test]
fn test_repay_rejects_during_flash_loan() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_repay(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}
#[test]
fn test_repay_cleans_up_empty_account() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.repay(ALICE, "ETH", 1.01);

    t.withdraw_all(ALICE, "USDC");

    let accounts = t.get_active_accounts(ALICE);
    assert_eq!(
        accounts.len(),
        0,
        "account should be auto-removed when empty"
    );
}
