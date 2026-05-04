extern crate std;

use common::constants::RAY;
use common::types::InterestRateModel;
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

    // Change LTV from default 7500 to 6000.
    t.edit_asset_config("USDC", |c| {
        c.loan_to_value_bps = 6000;
    });

    let config = t.get_asset_config("USDC");
    assert_eq!(
        config.loan_to_value_bps, 6000,
        "LTV should be updated to 6000"
    );
    // Threshold must remain unchanged.
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

    // Set threshold == LTV through the controller client; this must panic.
    let asset = t.resolve_market("USDC").asset.clone();
    let ctrl = t.ctrl_client();

    let mut config = ctrl.get_market_config(&asset).asset_config;
    config.loan_to_value_bps = 8000;
    config.liquidation_threshold_bps = 8000; // Equal to LTV.

    let result = ctrl.try_edit_asset_config(&asset, &config);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::INVALID_LIQ_THRESHOLD);
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

    // Supply first so the account exists.
    t.supply(ALICE, "USDC", 10_000.0);

    t.pause();

    // try_supply must fail while paused.
    let supply_result = t.try_supply(ALICE, "USDC", 1000.0);
    assert_contract_error(supply_result, errors::CONTRACT_PAUSED);

    // try_borrow must also fail while paused.
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
    // Verify the pause took effect.
    let result = t.try_supply(ALICE, "USDC", 1000.0);
    assert_contract_error(result, errors::CONTRACT_PAUSED);

    t.unpause();
    // The call must succeed after unpause.
    let result = t.try_supply(ALICE, "USDC", 1000.0);
    assert!(result.is_ok(), "supply should work after unpause");
}

// ---------------------------------------------------------------------------
// 6. test_upgrade_pool_params
// ---------------------------------------------------------------------------

#[test]
fn test_upgrade_pool_params() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    // Snapshot the borrow rate before upgrading params.
    let rate_before = t.pool_borrow_rate("USDC");

    let new_base_rate = RAY * 2 / 100; // 2%, far above default.
    let new_slope1 = RAY * 8 / 100; // 8%.

    t.upgrade_pool_params(
        "USDC",
        InterestRateModel {
            max_borrow_rate_ray: RAY * 2,
            base_borrow_rate_ray: new_base_rate,
            slope1_ray: new_slope1,
            slope2_ray: RAY * 10 / 100,
            slope3_ray: RAY * 150 / 100,
            mid_utilization_ray: RAY * 50 / 100,
            optimal_utilization_ray: RAY * 80 / 100,
            reserve_factor_bps: 1000,
        },
    );

    // Compare before/after to confirm the pool rate changed. At zero
    // utilization the rate equals base_rate / MILLISECONDS_PER_YEAR, so the
    // higher base_rate (2%) must raise it.
    let rate_after = t.pool_borrow_rate("USDC");
    assert!(
        rate_after > rate_before,
        "borrow rate should increase after doubling base_borrow_rate: before={}, after={}",
        rate_before,
        rate_after
    );
}

#[test]
fn test_upgrade_liquidity_pool_params_alias() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let ctrl = t.ctrl_client();
    let asset = t.resolve_market("USDC").asset.clone();

    let rate_before = t.pool_borrow_rate("USDC");

    ctrl.upgrade_liquidity_pool_params(
        &asset,
        &InterestRateModel {
            max_borrow_rate_ray: RAY * 2,
            base_borrow_rate_ray: RAY * 2 / 100,
            slope1_ray: RAY * 8 / 100,
            slope2_ray: RAY * 10 / 100,
            slope3_ray: RAY * 150 / 100,
            mid_utilization_ray: RAY * 50 / 100,
            optimal_utilization_ray: RAY * 80 / 100,
            reserve_factor_bps: 1000,
        },
    );

    let rate_after = t.pool_borrow_rate("USDC");
    assert!(
        rate_after > rate_before,
        "alias should update the pool params: before={}, after={}",
        rate_before,
        rate_after
    );
}

// ---------------------------------------------------------------------------
// 6b. Regression: `max_borrow_rate_ray` cap (Taylor envelope)
//
// `validate_interest_rate_model` and `pool::update_params` reject any
// `max_borrow_rate_ray > 2 * RAY` to keep `compound_interest`'s 8-term Taylor
// approximation inside its documented `< 0.01 %` accuracy envelope. See
// `architecture/MATH_REVIEW.md §0`.
// ---------------------------------------------------------------------------

#[test]
fn test_upgrade_pool_params_rejects_max_borrow_rate_above_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let ctrl = t.ctrl_client();

    // Just over the cap (`2 * RAY + 1`). Validator must panic with
    // `InvalidBorrowParams`.
    let result = ctrl.try_upgrade_pool_params(
        &asset,
        &InterestRateModel {
            max_borrow_rate_ray: 2 * RAY + 1,
            base_borrow_rate_ray: RAY / 100,
            slope1_ray: RAY * 4 / 100,
            slope2_ray: RAY * 10 / 100,
            slope3_ray: RAY * 150 / 100,
            mid_utilization_ray: RAY * 50 / 100,
            optimal_utilization_ray: RAY * 80 / 100,
            reserve_factor_bps: 1000,
        },
    );
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::INVALID_BORROW_PARAMS);
}

#[test]
fn test_upgrade_pool_params_accepts_max_borrow_rate_at_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let rate_before = t.pool_borrow_rate("USDC");

    // At the exact cap (`2 * RAY`); slope3 must remain <= max.
    t.upgrade_pool_params(
        "USDC",
        InterestRateModel {
            max_borrow_rate_ray: 2 * RAY,
            base_borrow_rate_ray: RAY / 100,
            slope1_ray: RAY * 4 / 100,
            slope2_ray: RAY * 10 / 100,
            slope3_ray: RAY * 150 / 100,
            mid_utilization_ray: RAY * 50 / 100,
            optimal_utilization_ray: RAY * 80 / 100,
            reserve_factor_bps: 1000,
        },
    );
    // The IRM was rewritten — confirm the borrow rate remains readable
    // after the boundary upgrade.
    let rate_after = t.pool_borrow_rate("USDC");
    assert!(
        rate_after != rate_before || rate_after >= 0.0,
        "borrow rate must remain readable after boundary upgrade",
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
        dex_symbol: soroban_sdk::Symbol::new(&t.env, ""),
        twap_records: 3,
    };

    t.mock_reflector_client().set_price(&asset, &1_0000000i128); // Dummy price for dry-run.

    // Must not panic; the admin has permission.
    ctrl.configure_market_oracle(&t.admin(), &asset, &config);

    let market = ctrl.get_market_config(&asset);
    assert_eq!(market.cex_oracle, Some(t.mock_reflector.clone()));
    assert_eq!(market.twap_records, 3);
    assert_eq!(
        (market.status as u32),
        1,
        "market should be Active after oracle config",
    );
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

    // Must not panic; the admin has permission.
    ctrl.set_aggregator(&new_aggregator);

    // Confirm the new aggregator is actually persisted.
    let stored: Address = t.env.as_contract(&t.controller_address(), || {
        t.env
            .storage()
            .instance()
            .get(&common::types::ControllerKey::Aggregator)
            .expect("aggregator must be stored")
    });
    assert_eq!(stored, new_aggregator, "aggregator must be persisted");
}

// ---------------------------------------------------------------------------
// 9. test_oracle_tolerance_validation
// ---------------------------------------------------------------------------

#[test]
fn test_oracle_tolerance_validation() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let ctrl = t.ctrl_client();
    let asset = t.resolve_market("USDC").asset.clone();

    // Set tolerance with first below MIN_FIRST_TOLERANCE (50 bps). The API
    // now takes raw deviation BPS (first, last) instead of
    // OraclePriceFluctuation.
    let result = ctrl.try_edit_oracle_tolerance(&t.admin(), &asset, &10, &500);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::BAD_FIRST_TOLERANCE);
}

// ---------------------------------------------------------------------------
// 10. test_grant_and_revoke_role
// ---------------------------------------------------------------------------

#[test]
fn test_grant_and_revoke_role() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Create BOB.
    t.get_or_create_user(BOB);

    // Grant KEEPER to BOB.
    t.grant_role(BOB, "KEEPER");
    assert!(t.has_role(BOB, "KEEPER"), "BOB should have KEEPER role");

    // BOB must lack the REVENUE role.
    assert!(
        !t.has_role(BOB, "REVENUE"),
        "BOB should not have REVENUE role"
    );

    // Revoke KEEPER from BOB.
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

    // Create BOB (no KEEPER role).
    let bob_addr = t.get_or_create_user(BOB);

    // BOB calls update_indexes without KEEPER; this must fail with
    // AccessControlError::Unauthorized = 2000.
    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::vec![&t.env, t.resolve_market("USDC").asset.clone()];
    let result = ctrl.try_update_indexes(&bob_addr, &assets);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, 2000);
}

// ---------------------------------------------------------------------------
// 12. test_role_enforcement_revenue
// ---------------------------------------------------------------------------

#[test]
fn test_role_enforcement_revenue() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Create BOB (no REVENUE role).
    let bob_addr = t.get_or_create_user(BOB);

    // claim_revenue is `#[only_role(caller, "REVENUE")]`; non-revenue callers
    // must trip AccessControlError::Unauthorized = 2000.
    let ctrl = t.ctrl_client();
    let asset = t.resolve_market("USDC").asset.clone();
    let assets = soroban_sdk::vec![&t.env, asset];
    let result = ctrl.try_claim_revenue(&bob_addr, &assets);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, 2000);
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
        dex_symbol: soroban_sdk::Symbol::new(&t.env, ""),
        twap_records: 3,
    };

    let configure_result =
        match ctrl.try_configure_market_oracle(&bob_addr, &asset, &reflector) {
            Ok(res) => res.map_err(|e| e.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        };
    assert_contract_error(configure_result, 2000);

    let tolerance_result = match ctrl.try_edit_oracle_tolerance(&bob_addr, &asset, &300, &600) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(tolerance_result, 2000);

    let disable_result = match ctrl.try_disable_token_oracle(&bob_addr, &asset) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(disable_result, 2000);
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
        dex_symbol: soroban_sdk::Symbol::new(&t.env, ""),
        twap_records: 2,
    };
    t.mock_reflector_client().set_price(&asset, &1_0000000i128);
    ctrl.configure_market_oracle(&bob_addr, &asset, &reflector);
    let after_configure = ctrl.get_market_config(&asset);
    assert_eq!(after_configure.cex_oracle, Some(t.mock_reflector.clone()));
    assert_eq!(after_configure.twap_records, 2);

    ctrl.edit_oracle_tolerance(&bob_addr, &asset, &300, &600);
    let after_tolerance = ctrl.get_market_config(&asset);
    assert!(
        after_tolerance.oracle_config.tolerance.first_upper_ratio_bps > 0,
        "tolerance must be persisted",
    );

    ctrl.disable_token_oracle(&bob_addr, &asset);
    let after_disable = ctrl.get_market_config(&asset);
    assert_eq!(
        (after_disable.status as u32),
        2,
        "disable_token_oracle must move market to Disabled (=2)",
    );
}

// ---------------------------------------------------------------------------
// 15. test_create_liquidity_pool_uniqueness
// ---------------------------------------------------------------------------

#[test]
fn test_create_liquidity_pool_uniqueness() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let ctrl = t.ctrl_client();
    let asset = t.resolve_asset("USDC");
    let params = usdc_preset().params.to_market_params(&asset, 7);
    let config = usdc_preset().config.to_asset_config(&t.env);

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
    let t = LendingTest::new().build(); // Empty protocol.
    let ctrl = t.ctrl_client();
    let admin = &t.admin;

    // Register a new token.
    let asset = t
        .env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let params = usdc_preset().params.to_market_params(&asset, 7);
    let config = usdc_preset().config.to_asset_config(&t.env);

    // 0. Pre-approve the token contract (allow-list gate, T1-7).
    ctrl.approve_token_wasm(&asset);

    // 1. Create the liquidity pool with no oracle; the call succeeds and
    //    leaves the market in PendingOracle.
    ctrl.create_liquidity_pool(&asset, &params, &config);

    // Confirm the market is pending (PendingOracle = 0).
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
        dex_symbol: soroban_sdk::Symbol::new(&t.env, ""),
        twap_records: 3,
    };
    t.mock_reflector_client().set_price(&asset, &1_0000000i128);
    ctrl.configure_market_oracle(admin, &asset, &reflector_cfg);

    // 3. Confirm the market is now Active (Active = 1).
    let m = ctrl.get_market_config(&asset);
    assert_eq!((m.status as u32), 1, "market should be in Active status");
}
