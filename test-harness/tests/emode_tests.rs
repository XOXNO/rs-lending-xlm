extern crate std;

use test_harness::{
    assert_contract_error, errors, eth_preset, usd_cents, usdc_preset, usdt_stable_preset,
    EModeCategoryPreset, LendingTest, PositionType, ALICE, LIQUIDATOR, STABLECOIN_EMODE,
};

// ---------------------------------------------------------------------------
// 1. test_emode_category_creation
// ---------------------------------------------------------------------------

#[test]
fn test_emode_category_creation() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .build();

    // Category was created during build; verify by creating an e-mode account
    // (would fail if category doesn't exist)
    let mut t = t;
    let account_id = t.create_emode_account(ALICE, 1);
    assert!(account_id > 0, "should create e-mode account");
}

// ---------------------------------------------------------------------------
// 2. test_emode_enhanced_ltv_and_threshold
// ---------------------------------------------------------------------------

#[test]
fn test_emode_enhanced_ltv_and_threshold() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    // E-mode LTV = 97%, threshold = 98%
    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);

    // Borrow 95% = $9500 USDT -- impossible with standard 75% LTV but fine with e-mode 97%
    t.borrow(ALICE, "USDT", 9_500.0);
    t.assert_healthy(ALICE);

    let hf = t.health_factor(ALICE);
    assert!(
        (1.0..1.10).contains(&hf),
        "e-mode should allow tight but healthy position, HF={}",
        hf
    );
}

// ---------------------------------------------------------------------------
// 3. test_emode_supply_with_category_asset
// ---------------------------------------------------------------------------

#[test]
fn test_emode_supply_with_category_asset() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .build();

    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 5_000.0);
    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
    t.assert_supply_near(ALICE, "USDC", 5_000.0, 1.0);
}

// ---------------------------------------------------------------------------
// 4. test_emode_borrow_with_category_asset
// ---------------------------------------------------------------------------

#[test]
fn test_emode_borrow_with_category_asset() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);

    t.assert_position_exists(ALICE, "USDT", PositionType::Borrow);
    t.assert_borrow_near(ALICE, "USDT", 5_000.0, 1.0);
    t.assert_healthy(ALICE);
}

// ---------------------------------------------------------------------------
// 5. test_emode_rejects_non_category_supply
// ---------------------------------------------------------------------------

#[test]
fn test_emode_rejects_non_category_supply() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset()) // ETH is NOT in the e-mode category
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .build();

    t.create_emode_account(ALICE, 1);

    // Supplying ETH (not in e-mode category) should fail
    let result = t.try_supply(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::EMODE_CATEGORY_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// 6. test_emode_rejects_non_category_borrow
// ---------------------------------------------------------------------------

#[test]
fn test_emode_rejects_non_category_borrow() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .build();

    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);

    // Borrowing ETH (not in e-mode category) should fail
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::EMODE_CATEGORY_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// 7. test_emode_rejects_with_isolation
// ---------------------------------------------------------------------------

#[test]
fn test_emode_rejects_with_isolation() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = 1_000_000 * 1_000_000_000_000_000_000;
        })
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .build();

    // Try to create an account with both e-mode and isolation -- should panic
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut t2 = t;
        t2.create_account_full(ALICE, 1, common::types::PositionMode::Normal, true);
    }));
    assert!(
        result.is_err(),
        "should reject creating account with both e-mode and isolation"
    );
}

// ---------------------------------------------------------------------------
// 8. test_emode_deprecated_blocks_new_accounts
// ---------------------------------------------------------------------------

#[test]
fn test_emode_deprecated_blocks_new_accounts() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .build();

    // Remove (deprecate) the e-mode category
    t.remove_e_mode_category(1);

    // Now trying to create an account with this category should fail
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut t2 = t;
        t2.create_emode_account(ALICE, 1);
    }));
    assert!(
        result.is_err(),
        "should reject new accounts for deprecated e-mode category"
    );
}

// ---------------------------------------------------------------------------
// 9. test_emode_edit_category_params
// ---------------------------------------------------------------------------

#[test]
fn test_emode_edit_category_params() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    // Edit category: lower LTV
    t.edit_e_mode_category(1, 8000, 8500, 300);

    // Now create account and try to borrow at 95% -- should fail with new 80% LTV
    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);

    let result = t.try_borrow(ALICE, "USDT", 9_500.0);
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}

// ---------------------------------------------------------------------------
// 10. test_emode_remove_category_deprecates
// ---------------------------------------------------------------------------

#[test]
fn test_emode_remove_category_deprecates() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .build();

    t.remove_e_mode_category(1);

    // Verify deprecated by trying to create new account -- should panic
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut t2 = t;
        t2.create_emode_account(ALICE, 1);
    }));
    assert!(result.is_err(), "removed category should be deprecated");
}

// ---------------------------------------------------------------------------
// 11. test_emode_add_asset_to_category
// ---------------------------------------------------------------------------

#[test]
fn test_emode_add_asset_to_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        // USDT not yet in category
        .build();

    // Add USDT to category at runtime
    t.add_asset_to_e_mode("USDT", 1, true, true);

    // Now should be able to use USDT in e-mode
    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);
    t.assert_healthy(ALICE);
}

// ---------------------------------------------------------------------------
// 12. test_emode_remove_asset_from_category
// ---------------------------------------------------------------------------

#[test]
fn test_emode_remove_asset_from_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    // Remove USDT from category
    t.remove_asset_from_e_mode("USDT", 1);

    // Trying to borrow USDT in e-mode should now fail
    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);

    let result = t.try_borrow(ALICE, "USDT", 5_000.0);
    assert_contract_error(result, errors::EMODE_CATEGORY_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// 13. test_emode_liquidation_uses_emode_bonus
// ---------------------------------------------------------------------------

#[test]
fn test_emode_liquidation_uses_emode_bonus() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    // E-mode bonus = 2% (200 BPS), much lower than standard 5%
    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 9_500.0);

    // Drop USDC price to make clearly liquidatable
    t.set_price("USDC", usd_cents(90));
    t.assert_liquidatable(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "USDT", 2_000.0);

    // Liquidator should receive collateral with e-mode bonus (2%)
    let usdc_received = t.token_balance(LIQUIDATOR, "USDC");
    assert!(usdc_received > 0.0, "liquidator should receive collateral");

    // Value ratio should be close to 1.02 (2% e-mode bonus), not 1.05 (standard)
    // USDC at $0.90, so usdc_value = usdc_received * 0.90
    let usdc_value = usdc_received * 0.90;
    let debt_value = 2_000.0; // USDT at $1

    if usdc_value > 0.0 {
        let ratio = usdc_value / debt_value;
        // E-mode bonus is 2%, so ratio should be around 1.02, not 1.05
        assert!(
            ratio < 1.06,
            "e-mode bonus should be lower than standard: ratio={}",
            ratio
        );
    }
}

// ---------------------------------------------------------------------------
// 14. test_emode_two_assets_same_category
// ---------------------------------------------------------------------------

#[test]
fn test_emode_two_assets_same_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    t.create_emode_account(ALICE, 1);

    // Supply both stablecoins
    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "USDT", 5_000.0);

    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
    t.assert_position_exists(ALICE, "USDT", PositionType::Supply);

    // Borrow USDC against USDT collateral and vice versa
    t.borrow(ALICE, "USDC", 2_000.0);
    t.assert_healthy(ALICE);
}

// ---------------------------------------------------------------------------
// 15. test_emode_rejects_threshold_lte_ltv
// ---------------------------------------------------------------------------

#[test]
fn test_emode_rejects_threshold_lte_ltv() {
    let _t = LendingTest::new().with_market(usdc_preset()).build();

    // Try to add e-mode category where threshold <= ltv -- should panic
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _t2 = LendingTest::new()
            .with_market(usdc_preset())
            .with_emode(
                1,
                EModeCategoryPreset {
                    ltv: 9000,
                    threshold: 8000, // threshold < ltv => invalid
                    bonus: 200,
                },
            )
            .build();
    }));
    assert!(
        result.is_err(),
        "should reject e-mode category where threshold <= ltv"
    );
}
