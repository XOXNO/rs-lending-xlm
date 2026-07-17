use soroban_sdk::{vec, Vec};
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, usdc_preset, HubAssetKey, LendingTest,
    PositionType, ALICE,
};

fn try_withdraw_payments(
    t: &mut LendingTest,
    user: &str,
    withdrawals: Vec<(HubAssetKey, i128)>,
) -> Result<Vec<(HubAssetKey, i128)>, soroban_sdk::Error> {
    let account_id = t.resolve_account_id(user);
    let caller = t.users.get(user).unwrap().address.clone();
    let ctrl = t.ctrl_client();

    match ctrl.try_withdraw(&caller, &account_id, &withdrawals, &None) {
        Ok(Ok(paid)) => Ok(paid),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

#[test]
fn test_withdraw_partial() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.withdraw(ALICE, "USDC", 3_000.0);

    // Supply must be ~7000.
    t.assert_supply_near(ALICE, "USDC", 7_000.0, 1.0);

    // The wallet must have received ~3000.
    let wallet = t.token_balance(ALICE, "USDC");
    assert!(
        wallet > 2_999.0 && wallet < 3_001.0,
        "wallet should have ~3000 USDC, got {}",
        wallet
    );
}
#[test]
fn test_withdraw_full_with_zero_amount() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.withdraw_all(ALICE, "USDC");

    // Supply balance must be 0.
    let supply = t.supply_balance(ALICE, "USDC");
    assert!(
        supply < 0.01,
        "supply should be ~0 after withdraw_all, got {}",
        supply
    );

    // Wallet must have ~10k back.
    let wallet = t.token_balance(ALICE, "USDC");
    assert!(
        wallet > 9_999.0,
        "wallet should have ~10k USDC, got {}",
        wallet
    );
}
#[test]
fn test_withdraw_multiple_assets() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 5.0);

    // Withdraw from both.
    t.withdraw(ALICE, "USDC", 2_000.0);
    t.withdraw(ALICE, "ETH", 1.0);

    t.assert_supply_near(ALICE, "USDC", 8_000.0, 1.0);
    t.assert_supply_near(ALICE, "ETH", 4.0, 0.01);
    t.assert_balance_eq(ALICE, "USDC", 2_000.0);
    t.assert_balance_eq(ALICE, "ETH", 1.0);
}
#[test]
fn test_withdraw_rejects_position_not_found() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);

    // Try to withdraw ETH: Alice has no ETH position.
    let result = t.try_withdraw(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::COLLATERAL_POSITION_NOT_FOUND);
}
#[test]
fn test_withdraw_rejects_exceeding_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    // Supply $10k, borrow $3500 ETH (1.75 ETH): near LTV.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.75);

    // Withdrawing $6000 USDC would leave only $4k collateral.
    // HF = (4000 * 0.80) / 3500 = 0.91 < 1.0: must fail.
    let result = t.try_withdraw(ALICE, "USDC", 6_000.0);
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}
#[test]
fn test_withdraw_allowed_without_borrows() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);

    // Full withdraw is OK when no borrows exist.
    t.withdraw_all(ALICE, "USDC");

    let supply = t.supply_balance(ALICE, "USDC");
    assert!(supply < 0.01, "supply should be ~0");
    t.assert_balance_eq(ALICE, "USDC", 10_000.0);
}
#[test]
fn test_withdraw_rejects_during_flash_loan() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}
#[test]
fn test_withdraw_allowed_when_paused() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.pause();

    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
    assert!(
        result.is_ok(),
        "withdraw should remain available while paused"
    );
}
#[test]
fn test_withdraw_removes_position_when_empty() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);

    // Withdraw all USDC.
    t.withdraw_all(ALICE, "USDC");

    // Only the ETH supply must remain.
    t.assert_supply_count(ALICE, 1);
    t.assert_position_exists(ALICE, "ETH", PositionType::Supply);
}
#[test]
fn test_withdraw_cleans_up_empty_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.withdraw_all(ALICE, "USDC");

    // The account was auto-removed by cleanup_account_if_empty when all
    // positions cleared.
    let accounts = t.get_active_accounts(ALICE);
    assert_eq!(
        accounts.len(),
        0,
        "account should be auto-removed when empty"
    );
}
#[test]
fn test_withdraw_full_amount_returned() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);

    // Wallet is 0 after supply.
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
#[test]
fn test_withdraw_raw_precision() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    // Supply 1000 USDC raw units.
    let supply_amount = 1000i128;
    t.supply_raw(ALICE, "USDC", supply_amount);

    // Withdraw 500 raw units.
    t.withdraw_raw(ALICE, "USDC", 500i128);

    let remaining = t.supply_balance_raw(ALICE, "USDC");
    // Must be approximately 500 (may differ slightly due to the index).
    assert!(
        (499..=501).contains(&remaining),
        "remaining supply should be ~500, got {}",
        remaining
    );
}
// Withdraw re-checks LTV-weighted collateral, not only LT health factor.
// USDC preset LTV 75% / LT 80%: $7500 ETH against $10k USDC is LTV-binding (HF ≈ 1.067);
// a 1-USDC withdraw must revert InsufficientCollateral even though HF would stay > 1.

#[test]
fn test_withdraw_rejects_when_above_ltv_but_hf_ok() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    // Supply $10k USDC. Borrow exactly at LTV: 7,500 / 2,000 = 3.75 ETH.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.75);

    // HF must be strictly above 1 — withdraw historically only saw this.
    let hf = t.health_factor(ALICE);
    assert!(
        hf > 1.0,
        "HF must be above 1 to expose the LTV-vs-HF gap, got {}",
        hf
    );

    // A tiny withdraw must now revert because the post-state would be above
    // LTV. Pre-fix this passed silently (HF stays above 1).
    let result = t.try_withdraw(ALICE, "USDC", 1.0);
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}
//
// Positive companion to test 13: when the borrow is below the LTV ceiling, a
// withdraw inside the headroom must succeed. Confirms the new LTV gate is
// strict-but-not-overzealous.

#[test]
fn test_withdraw_allowed_with_ltv_headroom() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    // Supply $10k USDC. Borrow 1 ETH = $2k → LTV-weighted = $7,500,
    // borrowed = $2,000, headroom = $5,500.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Withdrawing $1k USDC drops LTV-weighted to ~$6,750 — still well above
    // $2k debt. Must succeed.
    t.withdraw(ALICE, "USDC", 1_000.0);

    t.assert_supply_near(ALICE, "USDC", 9_000.0, 1.0);
}
#[test]
fn test_withdraw_to_pays_third_party_recipient() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let bob = t.get_or_create_user(test_harness::BOB);

    let alice_wallet_before = t.token_balance_raw(ALICE, "USDC");
    let bob_wallet_before = t.token_balance_raw(test_harness::BOB, "USDC");

    let paid = t.withdraw_to_raw(ALICE, "USDC", 30_000_000_000, &bob);
    let (_, paid_amount) = paid.get(0).unwrap();
    assert_eq!(paid_amount, 30_000_000_000);

    // Tokens land at the recipient; the owner's wallet is untouched.
    assert_eq!(
        t.token_balance_raw(test_harness::BOB, "USDC") - bob_wallet_before,
        30_000_000_000
    );
    assert_eq!(t.token_balance_raw(ALICE, "USDC"), alice_wallet_before);
    t.assert_supply_near(ALICE, "USDC", 7_000.0, 1.0);
}
#[test]
fn test_withdraw_returns_actual_amounts_on_full_close() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let wallet_before = t.token_balance_raw(ALICE, "USDC");

    // `0` sentinel closes the position; the returned amount is what the
    // pool actually paid (floor rounding of the position value).
    let paid = t.withdraw_raw_returning(ALICE, "USDC", 0);
    let (_, paid_amount) = paid.get(0).unwrap();

    assert!(
        (99_999_999_999..=100_000_000_000).contains(&paid_amount),
        "full close should pay ~10k USDC, got {paid_amount}"
    );
    assert_eq!(
        t.token_balance_raw(ALICE, "USDC") - wallet_before,
        paid_amount,
        "returned amount must equal the wallet delta"
    );
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), 0);
}
/// Debt-free full exit skips oracle (no LTV/HF; dust skips closed; events use cached prices).
#[test]
fn test_withdraw_full_exit_works_with_broken_oracle() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);

    // Any price resolution now panics with InvalidPrice (price 0).
    t.set_price("USDC", 0);

    t.withdraw_all(ALICE, "USDC");

    t.assert_balance_eq(ALICE, "USDC", 10_000.0);
    let accounts = t.get_active_accounts(ALICE);
    assert_eq!(accounts.len(), 0, "empty account should be auto-removed");
}
/// With debt, post-withdraw LTV/HF still requires a live oracle.
#[test]
fn test_withdraw_with_debt_still_requires_oracle() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 0.5);

    t.set_price("USDC", 0);

    let result = t.try_withdraw(ALICE, "USDC", 100.0);
    assert_contract_error(result, errors::INVALID_PRICE);
}

#[test]
fn test_withdraw_empty_vector_rejected() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let wallet_before = t.token_balance_raw(ALICE, "USDC");
    let supply_before = t.supply_balance_raw(ALICE, "USDC");

    let withdrawals = Vec::new(&t.env);
    let result = try_withdraw_payments(&mut t, ALICE, withdrawals);

    assert_contract_error(result, errors::INVALID_PAYMENTS);
    assert_eq!(t.token_balance_raw(ALICE, "USDC"), wallet_before);
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), supply_before);
    t.assert_supply_count(ALICE, 1);
}

#[test]
fn test_withdraw_aggregates_duplicate_assets() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let usdc = t.resolve_asset("USDC");
    let wallet_before = t.token_balance_raw(ALICE, "USDC");
    let withdrawals = vec![
        &t.env,
        (hub_asset(usdc.clone()), 10_000_000_000i128),
        (hub_asset(usdc), 25_000_000_000i128),
    ];

    let paid = try_withdraw_payments(&mut t, ALICE, withdrawals).unwrap();

    assert_eq!(paid.len(), 1, "duplicates should merge into one pool entry");
    let (_, paid_amount) = paid.get(0).unwrap();
    assert_eq!(paid_amount, 35_000_000_000);
    assert_eq!(
        t.token_balance_raw(ALICE, "USDC") - wallet_before,
        35_000_000_000
    );
    t.assert_supply_near(ALICE, "USDC", 6_500.0, 1.0);
}

#[test]
fn test_withdraw_duplicate_zero_full_close_is_sticky() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let usdc = t.resolve_asset("USDC");
    let wallet_before = t.token_balance_raw(ALICE, "USDC");
    let withdrawals = vec![
        &t.env,
        (hub_asset(usdc.clone()), 10_000_000_000i128),
        (hub_asset(usdc.clone()), 0i128),
        (hub_asset(usdc), 5_000_000_000i128),
    ];

    let paid = try_withdraw_payments(&mut t, ALICE, withdrawals).unwrap();

    assert_eq!(paid.len(), 1, "zero sentinel should stay aggregated");
    let (_, paid_amount) = paid.get(0).unwrap();
    assert!(
        (99_999_999_999..=100_000_000_000).contains(&paid_amount),
        "sticky zero should full-close and pay ~10k USDC, got {paid_amount}"
    );
    assert_eq!(
        t.token_balance_raw(ALICE, "USDC") - wallet_before,
        paid_amount
    );
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), 0);
    assert_eq!(t.get_active_accounts(ALICE).len(), 0);
}

#[test]
fn test_withdraw_rejects_negative_raw_amount() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let usdc = t.resolve_asset("USDC");
    let withdrawals = vec![&t.env, (hub_asset(usdc), -1i128)];

    let result = try_withdraw_payments(&mut t, ALICE, withdrawals);
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}
