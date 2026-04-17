extern crate std;

use common::constants::{RAY, WAD};
use test_harness::{
    eth_preset, usd_cents, usdc_preset, usdt_stable_preset, wbtc_preset, LendingTest, ALICE,
    STABLECOIN_EMODE,
};

// ---------------------------------------------------------------------------
// 1. test_total_collateral_usd_multi_asset
// ---------------------------------------------------------------------------

#[test]
fn test_total_collateral_usd_multi_asset() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply 10k USDC ($10,000) and 1 ETH ($2,000).
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);

    let total = t.total_collateral(ALICE);
    // Must be ~$12,000.
    assert!(
        (total - 12_000.0).abs() < 1.0,
        "total collateral should be ~$12,000, got {}",
        total
    );
}

// ---------------------------------------------------------------------------
// 2. test_total_borrow_usd_multi_asset
// ---------------------------------------------------------------------------

#[test]
fn test_total_borrow_usd_multi_asset() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    // Supply large collateral.
    t.supply(ALICE, "USDC", 500_000.0);

    // Borrow 1 ETH ($2,000) and 0.01 WBTC ($600).
    t.borrow(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "WBTC", 0.01);

    let total = t.total_debt(ALICE);
    // Must be ~$2,600.
    assert!(
        (total - 2_600.0).abs() < 1.0,
        "total debt should be ~$2,600, got {}",
        total
    );
}

// ---------------------------------------------------------------------------
// 3. test_collateral_amount_for_missing_token
// ---------------------------------------------------------------------------

#[test]
fn test_collateral_amount_for_missing_token() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply only USDC.
    t.supply(ALICE, "USDC", 10_000.0);

    // Check ETH collateral (no position): must return 0.
    let eth_balance = t.supply_balance_raw(ALICE, "ETH");
    assert_eq!(eth_balance, 0, "missing supply position should return 0");
}

// ---------------------------------------------------------------------------
// 4. test_borrow_amount_for_missing_token
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_amount_for_missing_token() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply USDC, borrow ETH.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Check the USDC borrow (no borrow position): must return 0.
    let usdc_borrow = t.borrow_balance_raw(ALICE, "USDC");
    assert_eq!(usdc_borrow, 0, "missing borrow position should return 0");
}

// ---------------------------------------------------------------------------
// 5. test_can_be_liquidated_boundary
// ---------------------------------------------------------------------------

#[test]
fn test_can_be_liquidated_boundary() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply 10k USDC, borrow conservatively so HF stays above 1.0.
    t.supply(ALICE, "USDC", 10_000.0);
    // Borrow $3000 of ETH = 1.5 ETH.
    // HF = (10000 * 0.80) / 3000 = 2.67: clearly healthy.
    t.borrow(ALICE, "ETH", 1.5);

    assert!(
        !t.can_be_liquidated(ALICE),
        "healthy account should not be liquidatable"
    );
    t.assert_healthy(ALICE);
}

// ---------------------------------------------------------------------------
// 6. test_can_be_liquidated_just_below
// ---------------------------------------------------------------------------

#[test]
fn test_can_be_liquidated_just_below() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply 10k USDC, borrow 3 ETH ($6000).
    // HF = (10000 * 0.80) / 6000 = 1.33.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.assert_healthy(ALICE);

    // Drop USDC to $0.50 => collateral = $5000, weighted = $4000, debt =
    // $6000. HF = 4000/6000 = 0.67 < 1.0.
    t.set_price("USDC", usd_cents(50));

    assert!(
        t.can_be_liquidated(ALICE),
        "undercollateralized account should be liquidatable"
    );
}

// ---------------------------------------------------------------------------
// 7. test_get_all_markets_multiple
// ---------------------------------------------------------------------------

#[test]
fn test_get_all_markets_multiple() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::Vec::from_array(
        &t.env,
        [
            t.resolve_asset("USDC"),
            t.resolve_asset("ETH"),
            t.resolve_asset("WBTC"),
        ],
    );
    let markets = ctrl.get_all_markets_detailed(&assets);
    assert_eq!(
        markets.len(),
        3,
        "should have 3 markets, got {}",
        markets.len()
    );
}

// ---------------------------------------------------------------------------
// 8. test_get_all_markets_empty_count
// ---------------------------------------------------------------------------

#[test]
fn test_get_all_markets_single() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::Vec::from_array(&t.env, [t.resolve_asset("USDC")]);
    let markets = ctrl.get_all_markets_detailed(&assets);
    assert_eq!(
        markets.len(),
        1,
        "should have 1 market, got {}",
        markets.len()
    );
}

// ---------------------------------------------------------------------------
// 9. test_get_account_owner_correct
// ---------------------------------------------------------------------------

#[test]
fn test_get_account_owner_correct() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 1_000.0);
    let account_id = t.resolve_account_id(ALICE);
    let alice_addr = t.users.get(ALICE).unwrap().address.clone();

    let owner = t.get_account_owner(account_id);
    assert_eq!(
        owner, alice_addr,
        "account owner should match Alice's address"
    );
}

// ---------------------------------------------------------------------------
// 10. test_get_emode_category_view
// ---------------------------------------------------------------------------

#[test]
fn test_get_emode_category_view() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    let ctrl = t.ctrl_client();
    let category = ctrl.get_e_mode_category(&1u32);

    // STABLECOIN_EMODE: ltv=9700, threshold=9800, bonus=200
    assert_eq!(category.loan_to_value_bps, 9700, "emode ltv should be 9700");
    assert_eq!(
        category.liquidation_threshold_bps, 9800,
        "emode threshold should be 9800"
    );
    assert_eq!(
        category.liquidation_bonus_bps, 200,
        "emode bonus should be 200"
    );
}

// ---------------------------------------------------------------------------
// 11. test_get_isolated_debt_tracks_borrows
// ---------------------------------------------------------------------------

#[test]
fn test_get_isolated_debt_tracks_borrows() {
    let isolation_ceiling = 1_000_000i128 * WAD;

    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = isolation_ceiling;
        })
        .with_market_config("USDC", |cfg| {
            cfg.isolation_borrow_enabled = true;
        })
        .build();

    // Create an isolated account and supply ETH.
    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 10.0);

    // Before borrow: isolated debt must be 0.
    let debt_before = t.get_isolated_debt("ETH");
    assert_eq!(debt_before, 0, "isolated debt should be 0 before borrow");

    // Borrow 1000 USDC ($1000).
    t.borrow(ALICE, "USDC", 1_000.0);

    // After borrow: isolated debt must be ~$1000 WAD.
    let debt_after = t.get_isolated_debt("ETH");
    let wad = WAD;
    assert!(
        debt_after > 999 * wad && debt_after < 1001 * wad,
        "isolated debt should be ~$1000, got {}",
        debt_after as f64 / wad as f64
    );
}

// ---------------------------------------------------------------------------
// 12. test_get_position_limits_default
// ---------------------------------------------------------------------------

#[test]
fn test_get_position_limits_default() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let limits = t.get_position_limits();
    assert_eq!(
        limits.max_supply_positions, 10,
        "default max supply should be 10"
    );
    assert_eq!(
        limits.max_borrow_positions, 10,
        "default max borrow should be 10"
    );
}

// ---------------------------------------------------------------------------
// 13. test_get_position_limits_custom
// ---------------------------------------------------------------------------

#[test]
fn test_get_position_limits_custom() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_position_limits(6, 3)
        .build();

    let limits = t.get_position_limits();
    assert_eq!(
        limits.max_supply_positions, 6,
        "custom max supply should be 6"
    );
    assert_eq!(
        limits.max_borrow_positions, 3,
        "custom max borrow should be 3"
    );
}

// ---------------------------------------------------------------------------
// 14. test_liquidation_estimations_basic
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_estimations_basic() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    // Make liquidatable: drop the USDC price.
    t.set_price("USDC", usd_cents(50));
    assert!(t.can_be_liquidated(ALICE));

    let account_id = t.resolve_account_id(ALICE);
    let ctrl = t.ctrl_client();
    let payments = soroban_sdk::Vec::from_array(&t.env, [(t.resolve_asset("ETH"), 3_0000000)]);
    let estimate = ctrl.liquidation_estimations_detailed(&account_id, &payments);
    let hf = ctrl.health_factor(&account_id);

    // HF must be < 1.0 WAD.
    let wad = WAD;
    assert!(hf < wad, "HF should be < 1.0 WAD, got {}", hf);
    assert!(hf > 0, "HF should be positive, got {}", hf);

    // Bonus must be positive.
    assert!(
        estimate.bonus_rate_bps > 0,
        "bonus should be positive, got {}",
        estimate.bonus_rate_bps
    );

    // Ideal repayment must be positive.
    assert!(
        estimate.max_payment_wad > 0,
        "ideal repayment should be positive, got {}",
        estimate.max_payment_wad
    );

    // Rich liquidation output should include seized collateral and fees.
    assert!(
        !estimate.seized_collaterals.is_empty(),
        "expected non-empty seized collateral estimate"
    );
    assert!(
        !estimate.protocol_fees.is_empty(),
        "expected non-empty protocol fee estimate"
    );
}

// ---------------------------------------------------------------------------
// 15. test_get_market_index_view
// ---------------------------------------------------------------------------

#[test]
fn test_get_market_index_view() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let asset = t.resolve_asset("USDC");
    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::Vec::from_array(&t.env, [asset]);
    let index = ctrl
        .get_all_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();

    let ray = RAY;
    // Fresh market: indexes must be 1.0 RAY.
    assert_eq!(
        index.supply_index_ray, ray,
        "fresh supply index should be 1.0 RAY"
    );
    assert_eq!(
        index.borrow_index_ray, ray,
        "fresh borrow index should be 1.0 RAY"
    );
}

// ---------------------------------------------------------------------------
// 16. test_get_active_accounts_multiple
// ---------------------------------------------------------------------------

#[test]
fn test_get_active_accounts_multiple() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Create two accounts for Alice.
    t.supply(ALICE, "USDC", 1_000.0);
    let id1 = t.resolve_account_id(ALICE);

    let id2 = t.create_account(ALICE);

    let accounts = t.get_active_accounts(ALICE);
    assert_eq!(accounts.len(), 2, "Alice should have 2 accounts");
    assert!(
        (accounts.get(0).unwrap() == id1 && accounts.get(1).unwrap() == id2)
            || (accounts.get(0).unwrap() == id2 && accounts.get(1).unwrap() == id1),
        "should contain both account IDs"
    );
}

// ---------------------------------------------------------------------------
// 17. test_get_asset_config_view
// ---------------------------------------------------------------------------

#[test]
fn test_get_asset_config_view() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let config = t.get_asset_config("USDC");
    assert_eq!(config.loan_to_value_bps, 7500, "LTV should be 7500");
    assert_eq!(
        config.liquidation_threshold_bps, 8000,
        "threshold should be 8000"
    );
    assert_eq!(config.liquidation_bonus_bps, 500, "bonus should be 500");
    assert!(
        config.is_collateralizable,
        "USDC should be collateralizable"
    );
    assert!(config.is_borrowable, "USDC should be borrowable");
}

// ---------------------------------------------------------------------------
// 18. test_pool_address_view
// ---------------------------------------------------------------------------

#[test]
fn test_pool_address_view() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let pool_addr = t.get_pool_address("USDC");
    let expected = t.resolve_market("USDC").pool.clone();
    assert_eq!(pool_addr, expected, "pool address should match");
}
