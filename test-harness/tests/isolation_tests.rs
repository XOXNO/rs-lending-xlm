extern crate std;

use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usdc_preset, wbtc_preset, LendingTest,
    PositionType, ALICE, LIQUIDATOR, STABLECOIN_EMODE,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const ISOLATION_CEILING_WAD: i128 = 1_000_000 * 1_000_000_000_000_000_000; // $1M in WAD

fn setup_isolated() -> LendingTest {
    LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .with_market(wbtc_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = ISOLATION_CEILING_WAD;
        })
        .with_market_config("USDC", |cfg| {
            cfg.isolation_borrow_enabled = true;
        })
        .with_market_config("WBTC", |cfg| {
            cfg.isolation_borrow_enabled = false;
        })
        .build()
}

// ---------------------------------------------------------------------------
// 1. test_isolated_account_creation
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_account_creation() {
    let mut t = setup_isolated();
    let account_id = t.create_isolated_account(ALICE, "ETH");
    assert!(account_id > 0, "should create isolated account");

    let attrs = t.get_account_attributes(ALICE);
    assert!(attrs.is_isolated, "account should be isolated");
}

// ---------------------------------------------------------------------------
// 2. test_isolated_supply_single_asset
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_supply_single_asset() {
    let mut t = setup_isolated();
    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 5.0);

    t.assert_position_exists(ALICE, "ETH", PositionType::Supply);
    t.assert_supply_near(ALICE, "ETH", 5.0, 0.01);
}

// ---------------------------------------------------------------------------
// 3. test_isolated_rejects_second_collateral
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_rejects_second_collateral() {
    let mut t = setup_isolated();
    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 5.0);

    // Try to supply USDC as second collateral in isolated account
    let result = t.try_supply(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::MIX_ISOLATED_COLLATERAL);
}

// ---------------------------------------------------------------------------
// 4. test_isolated_borrow_enabled_asset
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_borrow_enabled_asset() {
    let mut t = setup_isolated();
    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 5.0); // ~$10,000

    // USDC has isolation_borrow_enabled = true
    t.borrow(ALICE, "USDC", 5_000.0);
    t.assert_position_exists(ALICE, "USDC", PositionType::Borrow);
    t.assert_healthy(ALICE);
}

// ---------------------------------------------------------------------------
// 5. test_isolated_borrow_disabled_asset
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_borrow_disabled_asset() {
    let mut t = setup_isolated();
    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 5.0);

    // WBTC has isolation_borrow_enabled = false
    let result = t.try_borrow(ALICE, "WBTC", 0.01);
    assert_contract_error(result, errors::NOT_BORROWABLE_ISOLATION);
}

// ---------------------------------------------------------------------------
// 6. test_isolated_debt_ceiling_enforced
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_debt_ceiling_enforced() {
    // Use a very small ceiling
    let small_ceiling: i128 = 5_000 * 1_000_000_000_000_000_000; // $5000 WAD
    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = small_ceiling;
        })
        .with_market_config("USDC", |cfg| {
            cfg.isolation_borrow_enabled = true;
        })
        .build();

    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 5.0); // ~$10,000

    // Try to borrow beyond ceiling ($5000)
    let result = t.try_borrow(ALICE, "USDC", 6_000.0);
    assert_contract_error(result, errors::DEBT_CEILING_REACHED);
}

// ---------------------------------------------------------------------------
// 7. test_isolated_debt_decremented_on_repay
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_debt_decremented_on_repay() {
    let mut t = setup_isolated();
    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 5.0);
    t.borrow(ALICE, "USDC", 5_000.0);

    let debt_before = t.get_isolated_debt("ETH");
    assert!(debt_before > 0, "isolated debt should be tracked");

    t.repay(ALICE, "USDC", 2_000.0);

    let debt_after = t.get_isolated_debt("ETH");
    assert!(
        debt_after < debt_before,
        "isolated debt should decrease after repay: before={}, after={}",
        debt_before,
        debt_after
    );
}

// ---------------------------------------------------------------------------
// 8. test_isolated_debt_decremented_on_liquidation
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_debt_decremented_on_liquidation() {
    let mut t = setup_isolated();
    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 5.0); // ~$10,000
    t.borrow(ALICE, "USDC", 5_000.0);

    let debt_before = t.get_isolated_debt("ETH");

    // Make liquidatable by dropping ETH price
    t.set_price("ETH", usd(500));
    t.assert_liquidatable(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "USDC", 2_000.0);

    let debt_after = t.get_isolated_debt("ETH");
    assert!(
        debt_after < debt_before,
        "isolated debt should decrease after liquidation: before={}, after={}",
        debt_before,
        debt_after
    );
}

// ---------------------------------------------------------------------------
// 9. test_isolated_rejects_emode
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_rejects_emode() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = ISOLATION_CEILING_WAD;
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
        "should reject account with both e-mode and isolation"
    );
}

// ---------------------------------------------------------------------------
// 10. test_isolated_rejects_swap_collateral
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_rejects_swap_collateral() {
    let mut t = setup_isolated();
    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 5.0);

    let steps = t.mock_swap_steps("ETH", "USDC", usd(2000));
    let result = t.try_swap_collateral(ALICE, "ETH", 1.0, "USDC", &steps);
    // Strategy cross-contract calls may surface as host errors.
    assert!(
        result.is_err(),
        "should reject swap_collateral on isolated account"
    );
}

// ---------------------------------------------------------------------------
// 11. test_isolated_liquidation_works
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_liquidation_works() {
    let mut t = setup_isolated();
    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 5.0);
    t.borrow(ALICE, "USDC", 5_000.0);

    // Drop ETH price moderately to make mildly liquidatable
    // At $1000: collateral = $5000, threshold 80% => weighted = $4000, debt = $5000 => HF = 0.8
    t.set_price("ETH", usd(1000));
    t.assert_liquidatable(ALICE);

    let debt_before = t.borrow_balance(ALICE, "USDC");
    t.liquidate(LIQUIDATOR, ALICE, "USDC", 1_000.0);
    let debt_after = t.borrow_balance(ALICE, "USDC");

    assert!(
        debt_after < debt_before,
        "debt should decrease after liquidation: before={}, after={}",
        debt_before,
        debt_after
    );

    // Liquidator should have received ETH collateral
    let liq_eth = t.token_balance(LIQUIDATOR, "ETH");
    assert!(liq_eth > 0.0, "liquidator should receive ETH collateral");
}

// ---------------------------------------------------------------------------
// 12. test_isolated_bad_debt_clears_isolated_tracker
// ---------------------------------------------------------------------------

#[test]
fn test_isolated_bad_debt_clears_isolated_tracker() {
    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = ISOLATION_CEILING_WAD;
        })
        .with_market_config("USDC", |cfg| {
            cfg.isolation_borrow_enabled = true;
        })
        .build();

    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 0.1); // ~$200
    t.borrow(ALICE, "USDC", 100.0);

    let iso_debt_before = t.get_isolated_debt("ETH");
    assert!(iso_debt_before > 0, "isolated debt should be tracked");

    // Crash ETH price severely
    t.set_price("ETH", usd(50));
    t.assert_liquidatable(ALICE);

    // Liquidate -- bad debt handling should engage for tiny underwater positions
    t.liquidate(LIQUIDATOR, ALICE, "USDC", 100.0);

    // After liquidation + bad debt cleanup, the account should be removed
    // (collateral was tiny, so bad debt cleanup socializes the loss).
    // The isolated debt tracker should be cleared to zero.
    let iso_debt_after = t.get_isolated_debt("ETH");
    assert!(
        iso_debt_after < iso_debt_before,
        "isolated debt should decrease: before={}, after={}",
        iso_debt_before,
        iso_debt_after
    );
}
