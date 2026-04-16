extern crate std;

use common::constants::RAY;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Symbol};
use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, LendingTest, ALICE, BOB,
    DEFAULT_TOLERANCE,
};

// ---------------------------------------------------------------------------
// 1. test_edit_asset_config
// ---------------------------------------------------------------------------

#[test]
fn test_edit_asset_config() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    // Change LTV from default 7500 to 6000
    t.edit_asset_config("USDC", |c| {
        c.loan_to_value_bps = 6000;
    });

    let config = t.get_asset_config("USDC");
    assert_eq!(
        config.loan_to_value_bps, 6000,
        "LTV should be updated to 6000"
    );
    // Threshold should remain unchanged
    assert_eq!(
        config.liquidation_threshold_bps, 8000,
        "threshold should remain 8000"
    );
}

// ---------------------------------------------------------------------------
// 2. test_edit_asset_config_rejects_threshold_lte_ltv
// ---------------------------------------------------------------------------

#[test]
fn test_edit_asset_config_rejects_threshold_lte_ltv() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    // Set threshold == LTV directly via controller client (should panic)
    let asset = t.resolve_market("USDC").asset.clone();
    let ctrl = t.ctrl_client();

    let mut config = ctrl.get_market_config(&asset).asset_config;
    config.loan_to_value_bps = 8000;
    config.liquidation_threshold_bps = 8000; // equal to LTV

    let result = ctrl.try_edit_asset_config(&asset, &config);
    assert!(
        result.is_err(),
        "edit_asset_config should reject threshold == LTV"
    );
}

// ---------------------------------------------------------------------------
// 3. test_set_position_limits
// ---------------------------------------------------------------------------

#[test]
fn test_set_position_limits() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    t.set_position_limits(8, 6);

    let limits = t.get_position_limits();
    assert_eq!(limits.max_supply_positions, 8);
    assert_eq!(limits.max_borrow_positions, 6);
}

// ---------------------------------------------------------------------------
// 4. test_pause_blocks_operations
// ---------------------------------------------------------------------------

#[test]
fn test_pause_blocks_operations() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply first so account exists
    t.supply(ALICE, "USDC", 10_000.0);

    t.pause();

    // try_supply should fail when paused
    let supply_result = t.try_supply(ALICE, "USDC", 1000.0);
    assert_contract_error(supply_result, errors::CONTRACT_PAUSED);

    // try_borrow should also fail when paused
    let borrow_result = t.try_borrow(ALICE, "ETH", 0.5);
    assert_contract_error(borrow_result, errors::CONTRACT_PAUSED);
}

// ---------------------------------------------------------------------------
// 5. test_unpause_restores_operations
// ---------------------------------------------------------------------------

#[test]
fn test_unpause_restores_operations() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);

    t.pause();
    // Verify paused
    let result = t.try_supply(ALICE, "USDC", 1000.0);
    assert_contract_error(result, errors::CONTRACT_PAUSED);

    t.unpause();
    // Should succeed after unpause
    let result = t.try_supply(ALICE, "USDC", 1000.0);
    assert!(result.is_ok(), "supply should work after unpause");
}

// ---------------------------------------------------------------------------
// 6. test_upgrade_pool_params
// ---------------------------------------------------------------------------

#[test]
fn test_upgrade_pool_params() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    // Snapshot the borrow rate BEFORE upgrading params
    let rate_before = t.pool_borrow_rate("USDC");

    let new_base_rate = RAY * 2 / 100; // 2% (much higher than default)
    let new_slope1 = RAY * 8 / 100; // 8%

    t.upgrade_pool_params(
        "USDC",
        RAY * 5,         // max_borrow_rate (500% > slope3)
        new_base_rate,   // base_borrow_rate
        new_slope1,      // slope1
        RAY * 10 / 100,  // slope2
        RAY * 300 / 100, // slope3
        RAY * 50 / 100,  // mid_utilization
        RAY * 80 / 100,  // optimal_utilization
        1000,            // reserve_factor
    );

    // Verify the pool rate actually changed by comparing before/after.
    // With zero utilization, the rate = base_rate / MILLISECONDS_PER_YEAR.
    // The new base_rate (2%) is higher than the default, so the rate should increase.
    let rate_after = t.pool_borrow_rate("USDC");
    assert!(
        rate_after > rate_before,
        "borrow rate should increase after doubling base_borrow_rate: before={}, after={}",
        rate_before,
        rate_after
    );
}

// ---------------------------------------------------------------------------
// 7. test_configure_market_oracle
// ---------------------------------------------------------------------------

#[test]
fn test_configure_market_oracle() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let ctrl = t.ctrl_client();

    let asset = t.resolve_market("USDC").asset.clone();
    let new_oracle = t.mock_reflector.clone();

    let config = common::types::MarketOracleConfigInput {
        exchange_source: common::types::ExchangeSource::SpotVsTwap,
        max_price_stale_seconds: 900,
        first_tolerance_bps: 200,
        last_tolerance_bps: 500,
        cex_oracle: new_oracle,
        cex_asset_kind: common::types::ReflectorAssetKind::Stellar,
        cex_symbol: soroban_sdk::Symbol::new(&t.env, "USDC"),
        dex_oracle: None,
        dex_asset_kind: common::types::ReflectorAssetKind::Stellar,
        twap_records: 3,
    };

    t.mock_reflector_client().set_price(&asset, &1_0000000i128); // dummy price for dry-run

    // Should not panic -- admin has permission
    ctrl.configure_market_oracle(&t.admin(), &asset, &config);
}

// ---------------------------------------------------------------------------
// 8. test_set_aggregator
// ---------------------------------------------------------------------------

#[test]
fn test_set_aggregator() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let ctrl = t.ctrl_client();
    let new_aggregator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());

    // Should not panic -- admin has permission
    ctrl.set_aggregator(&new_aggregator);
}

// ---------------------------------------------------------------------------
// 9. test_oracle_tolerance_validation
// ---------------------------------------------------------------------------

#[test]
fn test_oracle_tolerance_validation() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let ctrl = t.ctrl_client();
    let asset = t.resolve_market("USDC").asset.clone();

    // Try setting tolerance with first below MIN_FIRST_TOLERANCE (50 bps).
    // API now takes raw deviation BPS (first, last) instead of OraclePriceFluctuation.
    let result = ctrl.try_edit_oracle_tolerance(&t.admin(), &asset, &10, &500);
    assert!(
        result.is_err(),
        "oracle tolerance with first < 50 bps should be rejected"
    );
}

// ---------------------------------------------------------------------------
// 10. test_grant_and_revoke_role
// ---------------------------------------------------------------------------

#[test]
fn test_grant_and_revoke_role() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Create BOB user
    t.get_or_create_user(BOB);

    // Grant KEEPER role to BOB
    t.grant_role(BOB, "KEEPER");
    assert!(t.has_role(BOB, "KEEPER"), "BOB should have KEEPER role");

    // BOB should NOT have REVENUE role
    assert!(
        !t.has_role(BOB, "REVENUE"),
        "BOB should not have REVENUE role"
    );

    // Revoke KEEPER from BOB
    t.revoke_role(BOB, "KEEPER");
    assert!(
        !t.has_role(BOB, "KEEPER"),
        "BOB should no longer have KEEPER role"
    );
}

// ---------------------------------------------------------------------------
// 11. test_role_enforcement_keeper
// ---------------------------------------------------------------------------

#[test]
fn test_role_enforcement_keeper() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Create BOB (no KEEPER role)
    let bob_addr = t.get_or_create_user(BOB);

    // BOB tries to call update_indexes without KEEPER role -- should fail.
    // Use bare `is_err()` because Soroban wraps cross-contract errors at the
    // outer caller boundary.
    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::vec![&t.env, t.resolve_market("USDC").asset.clone()];
    let result = ctrl.try_update_indexes(&bob_addr, &assets);
    assert!(
        result.is_err(),
        "non-keeper should not be able to call update_indexes"
    );
}

// ---------------------------------------------------------------------------
// 12. test_role_enforcement_revenue
// ---------------------------------------------------------------------------

#[test]
fn test_role_enforcement_revenue() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Create BOB (no REVENUE role)
    let bob_addr = t.get_or_create_user(BOB);

    // Use bare `is_err()` because Soroban wraps cross-contract errors at the
    // outer caller boundary.
    let ctrl = t.ctrl_client();
    let asset = t.resolve_market("USDC").asset.clone();
    let assets = soroban_sdk::vec![&t.env, asset];
    let result = ctrl.try_claim_revenue(&bob_addr, &assets);
    assert!(
        result.is_err(),
        "non-revenue user should not be able to claim revenue"
    );
}

// ---------------------------------------------------------------------------
// 13. test_role_enforcement_oracle
// ---------------------------------------------------------------------------

#[test]
fn test_role_enforcement_oracle() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let bob_addr = t.get_or_create_user(BOB);

    let ctrl = t.ctrl_client();
    let asset = t.resolve_market("USDC").asset.clone();
    let reflector = common::types::MarketOracleConfigInput {
        exchange_source: common::types::ExchangeSource::SpotVsTwap,
        max_price_stale_seconds: 900,
        first_tolerance_bps: 200,
        last_tolerance_bps: 500,
        cex_oracle: Address::generate(&t.env),
        cex_asset_kind: common::types::ReflectorAssetKind::Stellar,
        cex_symbol: soroban_sdk::Symbol::new(&t.env, "USDC"),
        dex_oracle: None,
        dex_asset_kind: common::types::ReflectorAssetKind::Stellar,
        twap_records: 3,
    };

    assert!(
        ctrl.try_configure_market_oracle(&bob_addr, &asset, &reflector)
            .is_err(),
        "non-oracle user should not be able to set reflector config"
    );
    assert!(
        ctrl.try_edit_oracle_tolerance(&bob_addr, &asset, &300, &600)
            .is_err(),
        "non-oracle user should not be able to edit oracle tolerance"
    );
    assert!(
        ctrl.try_disable_token_oracle(&bob_addr, &asset).is_err(),
        "non-oracle user should not be able to disable the oracle"
    );
}

// ---------------------------------------------------------------------------
// 14. test_oracle_role_can_manage_oracle_endpoints
// ---------------------------------------------------------------------------

#[test]
fn test_oracle_role_can_manage_oracle_endpoints() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.get_or_create_user(BOB);
    t.grant_role(BOB, "ORACLE");

    let bob_addr = t.users.get(BOB).unwrap().address.clone();
    let ctrl = t.ctrl_client();
    let asset = t.resolve_market("USDC").asset.clone();

    let reflector = common::types::MarketOracleConfigInput {
        exchange_source: common::types::ExchangeSource::SpotVsTwap,
        max_price_stale_seconds: 900,
        first_tolerance_bps: 200,
        last_tolerance_bps: 500,
        cex_oracle: t.mock_reflector.clone(),
        cex_asset_kind: common::types::ReflectorAssetKind::Stellar,
        cex_symbol: soroban_sdk::Symbol::new(&t.env, "USDC"),
        dex_oracle: None,
        dex_asset_kind: common::types::ReflectorAssetKind::Stellar,
        twap_records: 2,
    };
    t.mock_reflector_client().set_price(&asset, &1_0000000i128);
    ctrl.configure_market_oracle(&bob_addr, &asset, &reflector);

    ctrl.edit_oracle_tolerance(&bob_addr, &asset, &300, &600);

    let market = ctrl.get_market_config(&asset).asset_config;
    assert!(
        market.is_collateralizable,
        "asset config should remain readable"
    );

    ctrl.disable_token_oracle(&bob_addr, &asset);
}

// ---------------------------------------------------------------------------
// 15. test_init_market_uniqueness
// ---------------------------------------------------------------------------

#[test]
fn test_create_liquidity_pool_uniqueness() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let ctrl = t.ctrl_client();
    let asset = t.resolve_asset("USDC");
    let params = usdc_preset().params.to_market_params(&asset, 7);
    let config = usdc_preset().config.to_asset_config();

    // USDC was already initialized by the builder.
    // Calling create_liquidity_pool again should fail with AssetAlreadySupported.
    let result = match ctrl.try_create_liquidity_pool(&asset, &params, &config) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error")),
    };
    assert_contract_error(result, errors::ASSET_ALREADY_SUPPORTED);
}

// ---------------------------------------------------------------------------
// 16. test_market_initialization_cascade
// ---------------------------------------------------------------------------

#[test]
fn test_market_initialization_cascade() {
    let t = LendingTest::new().build(); // empty protocol
    let ctrl = t.ctrl_client();
    let admin = &t.admin;

    // Register a new token
    let asset = t
        .env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let params = usdc_preset().params.to_market_params(&asset, 7);
    let config = usdc_preset().config.to_asset_config();

    // 0. Pre-approve the token contract (allow-list gate, T1-7).
    ctrl.approve_token_wasm(&asset);

    // 1. Create liquidity pool without existing oracle -> Success (starts as PendingOracle)
    ctrl.create_liquidity_pool(&asset, &params, &config);

    // Verify market is pending (PendingOracle = 0)
    let m = ctrl.get_market_config(&asset);
    assert_eq!(
        (m.status as u32),
        0,
        "market should be in PendingOracle status"
    );

    // 2. Configure the full market oracle in one call.
    let reflector_cfg = common::types::MarketOracleConfigInput {
        exchange_source: common::types::ExchangeSource::SpotVsTwap,
        max_price_stale_seconds: 900,
        first_tolerance_bps: DEFAULT_TOLERANCE.first_upper_bps,
        last_tolerance_bps: DEFAULT_TOLERANCE.last_upper_bps,
        cex_oracle: t.mock_reflector.clone(),
        cex_asset_kind: common::types::ReflectorAssetKind::Stellar,
        cex_symbol: Symbol::new(&t.env, ""),
        dex_oracle: None,
        dex_asset_kind: common::types::ReflectorAssetKind::Stellar,
        twap_records: 3,
    };
    t.mock_reflector_client().set_price(&asset, &1_0000000i128);
    ctrl.configure_market_oracle(admin, &asset, &reflector_cfg);

    // 3. Verify market is now Active (Active = 1)
    let m = ctrl.get_market_config(&asset);
    assert_eq!((m.status as u32), 1, "market should be in Active status");
}
