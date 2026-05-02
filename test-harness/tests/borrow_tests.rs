extern crate std;

use common::constants::WAD;

use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, usdt_stable_preset, wbtc_preset,
    xlm_preset, LendingTest, PositionType, ALICE, STABLECOIN_EMODE,
};

// ---------------------------------------------------------------------------
// 1. test_borrow_basic
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 1b. test_borrow_same_asset_xlm
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 2. test_borrow_multiple_assets_bulk
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 3. test_borrow_rejects_exceeding_ltv
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 4. test_borrow_rejects_zero_amount
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_rejects_zero_amount() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);

    let result = t.try_borrow(ALICE, "ETH", 0.0);
    // Must reject with the precise AMOUNT_MUST_BE_POSITIVE (14). A generic
    // is_err() would hide regressions that fall through to a different
    // validator.
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

// ---------------------------------------------------------------------------
// 5. test_borrow_rejects_non_borrowable
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 6. test_borrow_rejects_during_flash_loan
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 7. test_borrow_rejects_when_paused
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 8. test_borrow_cap_enforcement
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_cap_enforcement() {
    let cap = 1_0000000i128; // 1 ETH in asset decimals (7 dec).
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("ETH", |cfg| {
            cfg.borrow_cap = cap;
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);

    // Borrowing 2 ETH exceeds the 1 ETH cap.
    let result = t.try_borrow(ALICE, "ETH", 2.0);
    assert_contract_error(result, errors::BORROW_CAP_REACHED);
}

// ---------------------------------------------------------------------------
// 9. test_borrow_position_limit_exceeded
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 10. test_borrow_siloed_asset_blocks_mixed
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_siloed_asset_blocks_mixed() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_siloed_borrowing = true;
        })
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 0.1);

    // Borrowing WBTC must fail because ETH is siloed.
    let result = t.try_borrow(ALICE, "WBTC", 0.001);
    assert_contract_error(result, errors::NOT_BORROWABLE_SILOED);
}

#[test]
fn test_borrow_bulk_rejects_siloed_asset_mixed_in_same_batch() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_siloed_borrowing = true;
        })
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);

    let result = t.try_borrow_bulk(ALICE, &[("ETH", 0.1), ("WBTC", 0.001)]);
    assert_contract_error(result, errors::NOT_BORROWABLE_SILOED);
    t.assert_borrow_count(ALICE, 0);
}

// ---------------------------------------------------------------------------
// 11. test_borrow_isolated_requires_enabled
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_isolated_requires_enabled() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market_config("USDC", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = 1_000_000i128 * WAD;
        })
        .with_market(eth_preset())
        .with_market_config("ETH", |cfg| {
            cfg.isolation_borrow_enabled = false;
        })
        .build();

    t.create_isolated_account(ALICE, "USDC");
    t.supply(ALICE, "USDC", 10_000.0);

    // ETH lacks isolation_borrow_enabled, so this must fail.
    let result = t.try_borrow(ALICE, "ETH", 0.1);
    assert_contract_error(result, errors::NOT_BORROWABLE_ISOLATION);
}

// ---------------------------------------------------------------------------
// 12. test_borrow_isolated_debt_ceiling
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_isolated_debt_ceiling() {
    // Set a very low ceiling: $100 WAD.
    let ceiling = 100 * WAD;
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

    // Borrowing 1 ETH = $2000 must exceed the $100 ceiling.
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::DEBT_CEILING_REACHED);
}

// ---------------------------------------------------------------------------
// 13. test_borrow_emode_enhanced_ltv
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 14. test_borrow_health_factor_exactly_one
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_health_factor_exactly_one() {
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

// ---------------------------------------------------------------------------
// 15. test_borrow_bulk_passes_cumulative_hf_check
// ---------------------------------------------------------------------------

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
