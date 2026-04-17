extern crate std;

use common::constants::WAD;

use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, usdt_stable_preset, wbtc_preset,
    LendingTest, PositionType, ALICE, STABLECOIN_EMODE,
};

// ---------------------------------------------------------------------------
// 1. test_supply_single_asset
// ---------------------------------------------------------------------------

#[test]
fn test_supply_single_asset() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);

    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);

    // Wallet must be ~0.
    let wallet = t.token_balance(ALICE, "USDC");
    assert!(
        wallet < 0.01,
        "wallet should be ~0 after supply, got {}",
        wallet
    );

    // Supply balance must be ~10_000.
    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);

    // Total collateral in USD must be ~$10k.
    let coll = t.total_collateral(ALICE);
    assert!(
        coll > 9_999.0 && coll < 10_001.0,
        "collateral should be ~$10k, got {}",
        coll
    );
}

// ---------------------------------------------------------------------------
// 2. test_supply_to_existing_account
// ---------------------------------------------------------------------------

#[test]
fn test_supply_to_existing_account() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 5_000.0);
    t.assert_supply_near(ALICE, "USDC", 5_000.0, 1.0);

    // Supply more to the same account.
    t.supply(ALICE, "USDC", 3_000.0);
    t.assert_supply_near(ALICE, "USDC", 8_000.0, 1.0);
}

// ---------------------------------------------------------------------------
// 3. test_supply_multiple_assets_bulk
// ---------------------------------------------------------------------------

#[test]
fn test_supply_multiple_assets_bulk() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Bulk supply via the harness method: a single controller call that
    // auto-mints.
    t.supply_bulk(ALICE, &[("USDC", 10_000.0), ("ETH", 1.0)]);

    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
    t.assert_position_exists(ALICE, "ETH", PositionType::Supply);
}

// ---------------------------------------------------------------------------
// 4. test_supply_creates_account_on_first_call
// ---------------------------------------------------------------------------

#[test]
fn test_supply_creates_account_on_first_call() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // No explicit create_account: supply auto-creates.
    t.supply(ALICE, "USDC", 1_000.0);

    let accounts = t.get_active_accounts(ALICE);
    assert_eq!(accounts.len(), 1, "supply should auto-create an account");
    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
}

// ---------------------------------------------------------------------------
// 5. test_supply_with_emode_category
// ---------------------------------------------------------------------------

#[test]
fn test_supply_with_emode_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);

    let attrs = t.get_account_attributes(ALICE);
    assert_eq!(attrs.e_mode_category_id, 1);
    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
}

// ---------------------------------------------------------------------------
// 6. test_supply_rejects_zero_amount
// ---------------------------------------------------------------------------

#[test]
fn test_supply_rejects_zero_amount() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.create_account(ALICE);

    let result = t.try_supply(ALICE, "USDC", 0.0);
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

// ---------------------------------------------------------------------------
// 7. test_supply_rejects_non_collateralizable
// ---------------------------------------------------------------------------

#[test]
fn test_supply_rejects_non_collateralizable() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market_config("USDC", |cfg| {
            cfg.is_collateralizable = false;
        })
        .build();

    let result = t.try_supply(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::NOT_COLLATERAL);
}

// ---------------------------------------------------------------------------
// 8. test_supply_rejects_during_flash_loan
// ---------------------------------------------------------------------------

#[test]
fn test_supply_rejects_during_flash_loan() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.set_flash_loan_ongoing(true);

    let result = t.try_supply(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}

// ---------------------------------------------------------------------------
// 9. test_supply_rejects_when_paused
// ---------------------------------------------------------------------------

#[test]
fn test_supply_rejects_when_paused() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.pause();

    let result = t.try_supply(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::CONTRACT_PAUSED);
}

// ---------------------------------------------------------------------------
// 10. test_supply_cap_enforcement
// ---------------------------------------------------------------------------

#[test]
fn test_supply_cap_enforcement() {
    // Set a low supply cap of 500 tokens (7 decimals).
    let cap = 500_0000000i128; // 500 tokens in asset decimals
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market_config("USDC", |cfg| {
            cfg.supply_cap = cap;
        })
        .build();

    // Supply 600 USDC: must exceed the 500-token cap.
    let result = t.try_supply(ALICE, "USDC", 600.0);
    assert_contract_error(result, errors::SUPPLY_CAP_REACHED);
}

// ---------------------------------------------------------------------------
// 11. test_supply_position_limit_exceeded
// ---------------------------------------------------------------------------

#[test]
fn test_supply_position_limit_exceeded() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_position_limits(2, 2)
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(ALICE, "ETH", 1.0);

    // The third supply must exceed the limit of 2. Note: the Soroban host
    // wraps the error as InvalidAction on the cross-contract path.
    let result = t.try_supply(ALICE, "WBTC", 0.01);
    assert!(
        result.is_err(),
        "supply exceeding position limit should fail"
    );
}

// ---------------------------------------------------------------------------
// 12. test_supply_isolated_account_single_asset
// ---------------------------------------------------------------------------

#[test]
fn test_supply_isolated_account_single_asset() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market_config("USDC", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = 1_000_000i128 * WAD;
        })
        .with_market(eth_preset())
        .build();

    t.create_isolated_account(ALICE, "USDC");
    t.supply(ALICE, "USDC", 5_000.0);

    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
    t.assert_supply_near(ALICE, "USDC", 5_000.0, 1.0);
}

// ---------------------------------------------------------------------------
// 13. test_supply_isolated_rejects_second_asset
// ---------------------------------------------------------------------------

#[test]
fn test_supply_isolated_rejects_second_asset() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market_config("USDC", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = 1_000_000i128 * WAD;
        })
        .with_market(eth_preset())
        .build();

    t.create_isolated_account(ALICE, "USDC");
    t.supply(ALICE, "USDC", 5_000.0);

    // Supplying ETH to an isolated account must fail.
    let result = t.try_supply(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::MIX_ISOLATED_COLLATERAL);
}

// ---------------------------------------------------------------------------
// 14. test_supply_emode_rejects_non_category_asset
// ---------------------------------------------------------------------------

#[test]
fn test_supply_emode_rejects_non_category_asset() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        // ETH is NOT in the e-mode category
        .build();

    t.create_emode_account(ALICE, 1);

    // Supplying ETH to an e-mode stablecoin account must fail.
    let result = t.try_supply(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::EMODE_CATEGORY_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// 15. test_supply_raw_precision
// ---------------------------------------------------------------------------

#[test]
fn test_supply_raw_precision() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Supply exactly 1 unit (smallest: 1 with 7 decimals = 0.0000001 USDC).
    let raw_amount = 1i128;
    t.supply_raw(ALICE, "USDC", raw_amount);

    let balance = t.supply_balance_raw(ALICE, "USDC");
    // Must be at least 1 (could be exactly 1 or close due to the index).
    assert!(
        balance >= 1,
        "raw supply should preserve precision, got {}",
        balance
    );
}
