use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, usdt_stable_preset, wbtc_preset,
    xlm_preset, LendingTest, PositionType, ALICE, BOB, STABLECOIN_EMODE,
};
// 1. test_borrow_basic

#[test]
fn test_borrow_basic() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
    t.assert_borrow_near(ALICE, "ETH", 1.0, 0.01);
    t.assert_healthy(ALICE);

    // The wallet must hold the borrowed ETH.
    let eth_wallet = t.token_balance(ALICE, "ETH");
    assert!(
        eth_wallet > 0.99,
        "should have ~1 ETH in wallet, got {}",
        eth_wallet
    );
}
// 1b. test_borrow_same_asset_xlm

#[test]
fn test_borrow_same_asset_xlm() {
    let mut t = LendingTest::new().with_market(xlm_preset()).build();

    // Supply 1,000,000 XLM ($100,000), then borrow 500,000 XLM ($50,000).
    t.supply(ALICE, "XLM", 1_000_000.0);
    t.borrow(ALICE, "XLM", 500_000.0);

    t.assert_position_exists(ALICE, "XLM", PositionType::Supply);
    t.assert_position_exists(ALICE, "XLM", PositionType::Borrow);
    t.assert_supply_near(ALICE, "XLM", 1_000_000.0, 10.0);
    t.assert_borrow_near(ALICE, "XLM", 500_000.0, 10.0);
    t.assert_healthy(ALICE);

    let hf = t.health_factor(ALICE);
    assert!(
        hf > 1.5,
        "HF should remain safely above 1.0 for same-asset XLM borrow, got {}",
        hf
    );
}
// 2. test_borrow_multiple_assets_bulk

#[test]
fn test_borrow_multiple_assets_bulk() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    // Supply enough collateral.
    t.supply(ALICE, "USDC", 100_000.0);

    // Borrow 1 ETH ($2000) and 0.01 WBTC ($600) in one bulk call.
    t.borrow_bulk(ALICE, &[("ETH", 1.0), ("WBTC", 0.01)]);

    t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
    t.assert_position_exists(ALICE, "WBTC", PositionType::Borrow);
    t.assert_borrow_near(ALICE, "ETH", 1.0, 0.01);
    t.assert_borrow_near(ALICE, "WBTC", 0.01, 0.0001);
    let eth_wallet = t.token_balance(ALICE, "ETH");
    let wbtc_wallet = t.token_balance(ALICE, "WBTC");
    assert!(
        eth_wallet > 0.99,
        "ETH wallet should be ~1.0, got {}",
        eth_wallet
    );
    assert!(
        wbtc_wallet > 0.0099,
        "WBTC wallet should be ~0.01, got {}",
        wbtc_wallet
    );
    t.assert_healthy(ALICE);
}

#[test]
fn test_borrow_duplicate_asset_bulk_accumulates_single_position() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow_bulk(ALICE, &[("ETH", 0.5), ("ETH", 0.75)]);

    t.assert_borrow_count(ALICE, 1);
    t.assert_borrow_near(ALICE, "ETH", 1.25, 0.01);
    let eth_wallet = t.token_balance(ALICE, "ETH");
    assert!(
        eth_wallet > 1.24 && eth_wallet < 1.26,
        "ETH wallet should be ~1.25 after duplicate-asset borrow, got {}",
        eth_wallet
    );
    t.assert_healthy(ALICE);
}
// 3. test_borrow_rejects_exceeding_ltv

#[test]
fn test_borrow_rejects_exceeding_ltv() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply $10k. LTV = 75%, so max borrow value = $7500.
    // ETH = $2000, so max borrow ~ 3.75 ETH.
    t.supply(ALICE, "USDC", 10_000.0);

    // Borrow 5 ETH = $10k, which must exceed the LTV.
    let result = t.try_borrow(ALICE, "ETH", 5.0);
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}
// 4. test_borrow_rejects_zero_amount

#[test]
fn test_borrow_rejects_zero_amount() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);

    let result = t.try_borrow(ALICE, "ETH", 0.0);
    // Must reject with the precise AMOUNT_MUST_BE_POSITIVE (14), not a generic
    // validator failure.
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}
// 5. test_borrow_rejects_non_borrowable

#[test]
fn test_borrow_rejects_non_borrowable() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_borrowable = false;
        })
        .build();

    t.supply(ALICE, "USDC", 10_000.0);

    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::ASSET_NOT_BORROWABLE);
}
// 6. test_borrow_rejects_during_flash_loan

#[test]
fn test_borrow_rejects_during_flash_loan() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}
// 7. test_borrow_rejects_when_paused

#[test]
fn test_borrow_rejects_when_paused() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.pause();

    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::CONTRACT_PAUSED);
}
// 8. test_borrow_cap_enforcement

#[test]
fn test_borrow_cap_enforcement() {
    let cap = 1_0000000i128; // 1 ETH in asset decimals (7 dec).
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_params("ETH", |params| {
            params.borrow_cap = cap;
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);

    // Borrowing 2 ETH exceeds the 1 ETH cap.
    let result = t.try_borrow(ALICE, "ETH", 2.0);
    assert_contract_error(result, errors::BORROW_CAP_REACHED);
}
// 9. test_borrow_position_limit_exceeded

#[test]
fn test_borrow_position_limit_exceeded() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_position_limits(4, 1) // Only one borrow position allowed.
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 0.1);

    // The second borrow position must exceed the limit.
    let result = t.try_borrow(ALICE, "WBTC", 0.001);
    assert_contract_error(result, errors::POSITION_LIMIT_EXCEEDED);
}
// 10. test_borrow_emode_enhanced_ltv

#[test]
fn test_borrow_emode_enhanced_ltv() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);

    // Standard LTV = 75% caps the normal limit at $7500.
    // E-mode LTV = 97%, so a $9500 borrow stays allowed.
    t.borrow(ALICE, "USDT", 9_500.0);
    t.assert_position_exists(ALICE, "USDT", PositionType::Borrow);
    t.assert_borrow_near(ALICE, "USDT", 9_500.0, 1.0);
    let usdt_wallet = t.token_balance(ALICE, "USDT");
    assert!(
        usdt_wallet > 9_499.0,
        "USDT wallet should be ~9500, got {}",
        usdt_wallet
    );
    t.assert_healthy(ALICE);

    let hf = t.health_factor(ALICE);
    assert!(hf >= 1.0, "should be healthy with e-mode LTV, HF = {}", hf);
}
// 14. test_borrow_at_ltv_limit_stays_healthy

#[test]
fn test_borrow_at_ltv_limit_stays_healthy() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .build();

    // Supply $10k USDC. LTV = 75%, so max borrow = $7500.
    t.supply(ALICE, "USDC", 10_000.0);

    // Borrow at the LTV limit: $7500 USDT.
    // HF = (10_000 * 0.80) / 7500 = 1.0667 -- healthy but tight.
    // HF uses liquidation_threshold (80%), not LTV (75%).
    t.borrow(ALICE, "USDT", 7_500.0);
    t.assert_healthy(ALICE);
    let usdt_wallet = t.token_balance(ALICE, "USDT");
    assert!(
        usdt_wallet > 7_499.0,
        "USDT wallet should hold ~7500, got {}",
        usdt_wallet
    );

    let hf = t.health_factor(ALICE);
    assert!(
        (1.0..1.15).contains(&hf),
        "HF should be tight (~1.07), got {}",
        hf
    );
}
// 15. test_borrow_bulk_passes_cumulative_hf_check

#[test]
fn test_borrow_bulk_passes_cumulative_hf_check() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    // Supply enough collateral.
    t.supply(ALICE, "USDC", 100_000.0);

    // Borrow small amounts of each in one batch through the harness.
    t.borrow_bulk(ALICE, &[("ETH", 0.5), ("WBTC", 0.005)]);

    t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
    t.assert_position_exists(ALICE, "WBTC", PositionType::Borrow);
    t.assert_borrow_near(ALICE, "ETH", 0.5, 0.01);
    t.assert_borrow_near(ALICE, "WBTC", 0.005, 0.0001);
    let eth_wallet = t.token_balance(ALICE, "ETH");
    let wbtc_wallet = t.token_balance(ALICE, "WBTC");
    assert!(eth_wallet > 0.49, "ETH wallet ~0.5, got {}", eth_wallet);
    assert!(
        wbtc_wallet > 0.0049,
        "WBTC wallet ~0.005, got {}",
        wbtc_wallet
    );
    t.assert_healthy(ALICE);
}
// 16. test_delegated_borrow_routes_funds_to_owner

#[test]
fn test_delegated_borrow_routes_funds_to_owner() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // ALICE owns the account and the collateral; BOB is her delegate.
    t.supply(ALICE, "USDC", 10_000.0);
    let account_id = t.resolve_account_id(ALICE);
    t.enable_delegate(ALICE, BOB, account_id);

    let alice_before = t.token_balance(ALICE, "ETH");
    let bob_before = t.token_balance(BOB, "ETH");

    // BOB borrows on ALICE's account, routing the funds to ALICE via `to`.
    t.borrow_as_to(BOB, account_id, "ETH", 1.0, ALICE);

    let alice_gain = t.token_balance(ALICE, "ETH") - alice_before;
    let bob_gain = t.token_balance(BOB, "ETH") - bob_before;

    // Funds land on the owner, not the delegate caller.
    assert!(alice_gain > 0.99, "owner should receive ~1 ETH, got {}", alice_gain);
    assert!(bob_gain < 0.01, "delegate must receive nothing, got {}", bob_gain);

    // Debt is recorded on the account regardless of destination.
    t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
    t.assert_borrow_near(ALICE, "ETH", 1.0, 0.01);
    t.assert_healthy(ALICE);
}
// 17. test_delegated_borrow_to_none_routes_to_caller

#[test]
fn test_delegated_borrow_to_none_routes_to_caller() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let account_id = t.resolve_account_id(ALICE);
    t.enable_delegate(ALICE, BOB, account_id);

    let alice_before = t.token_balance(ALICE, "ETH");
    let bob_before = t.token_balance(BOB, "ETH");

    // `to = None` keeps today's behavior: funds go to the caller (BOB here).
    t.borrow_to(BOB, account_id, "ETH", 1.0);

    let alice_gain = t.token_balance(ALICE, "ETH") - alice_before;
    let bob_gain = t.token_balance(BOB, "ETH") - bob_before;

    assert!(bob_gain > 0.99, "caller should receive ~1 ETH, got {}", bob_gain);
    assert!(alice_gain.abs() < 0.01, "owner wallet must be unchanged, got {}", alice_gain);

    // Debt still lands on the account, not the caller.
    t.assert_borrow_near(ALICE, "ETH", 1.0, 0.01);
    t.assert_healthy(ALICE);
}
