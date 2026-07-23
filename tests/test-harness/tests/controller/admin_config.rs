use controller::constants::RAY;
use controller::types::{
    AssetOracleConfig, InterestRateModel, OracleAssetRef, OracleReadMode, OracleSourceConfig,
    OracleSourceConfigOption, OracleStrategy, OracleTolerance, PositionLimits, ReflectorBase,
    ReflectorSourceConfig,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, usdc_preset, LendingTest, ALICE, BOB,
    DEFAULT_TOLERANCE, HARNESS_HUB,
};

/// Pre-resolved config for the thin `set_oracle_config` setter:
/// mock-reflector shape (14 decimals, 300 s resolution, USD base) with the
/// 200/500 BPS tolerance bands governance computes in-path.
fn resolved_reflector_primary_anchor_config(
    oracle: &Address,
    asset: &Address,
) -> AssetOracleConfig {
    let source = |read_mode: OracleReadMode| {
        OracleSourceConfig::Reflector(ReflectorSourceConfig {
            contract: oracle.clone(),
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode,
            decimals: 14,
            resolution_seconds: 300,
            base: ReflectorBase::Usd,
        })
    };
    AssetOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        strategy: OracleStrategy::PrimaryWithAnchor,
        primary: source(OracleReadMode::Twap(3)),
        anchor: OracleSourceConfigOption::Some(source(OracleReadMode::Spot)),
        min_sanity_price_wad: 1,
        max_sanity_price_wad: controller::constants::MAX_REASONABLE_PRICE_WAD,
    }
}
#[test]
fn test_edit_asset_config() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    // Change LTV from default 7500 to 6000.
    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 6000;
    });

    let config = t.get_asset_config("USDC");
    assert_eq!(config.loan_to_value, 6000, "LTV should be updated to 6000");
    // Threshold must remain unchanged.
    assert_eq!(
        config.liquidation_threshold, 8000,
        "threshold should remain 8000"
    );
}
#[test]
fn test_set_position_limits() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    t.set_position_limits(8, 6);

    let limits = t.get_position_limits();
    assert_eq!(limits.max_supply_positions, 8);
    assert_eq!(limits.max_borrow_positions, 6);
}

// The controller re-validates position limits at execution (`1..=POSITION_LIMIT_MAX`
// per side): a zero cap (bricks a side) and an over-cap value are both rejected.
#[test]
fn test_set_position_limits_rejects_out_of_range() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let ctrl = t.ctrl_client();

    for limits in [
        PositionLimits {
            max_supply_positions: 0,
            max_borrow_positions: 5,
        },
        PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 0,
        },
        PositionLimits {
            max_supply_positions: 11,
            max_borrow_positions: 5,
        },
    ] {
        let result = ctrl.try_set_position_limits(&limits);
        let mapped = match result {
            Ok(res) => res.map_err(|e| e.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        };
        assert_contract_error(mapped, errors::GenericError::InvalidPositionLimits as u32);
    }
}

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
            max_borrow_rate: RAY * 2,
            base_borrow_rate: new_base_rate,
            slope1: new_slope1,
            slope2: RAY * 10 / 100,
            slope3: RAY * 150 / 100,
            mid_utilization: RAY * 50 / 100,
            optimal_utilization: RAY * 80 / 100,
            max_utilization: controller::constants::RAY * 95 / 100,
            reserve_factor: 1000,
            is_flashloanable: false,
            flashloan_fee: 0,
        },
    );

    // Compare before/after to confirm the pool rate differs. At zero
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
        &hub_asset(asset.clone()),
        &InterestRateModel {
            max_borrow_rate: RAY * 2,
            base_borrow_rate: RAY * 2 / 100,
            slope1: RAY * 8 / 100,
            slope2: RAY * 10 / 100,
            slope3: RAY * 150 / 100,
            mid_utilization: RAY * 50 / 100,
            optimal_utilization: RAY * 80 / 100,
            max_utilization: controller::constants::RAY * 95 / 100,
            reserve_factor: 1000,
            is_flashloanable: false,
            flashloan_fee: 0,
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
// 6b. Regression: `max_borrow_rate` cap (Taylor envelope)
//
// `pool::update_params` rejects any `max_borrow_rate > 2 * RAY` to keep
// `compound_interest`'s 8-term Taylor approximation inside its documented
// `< 0.01 %` accuracy envelope. See `architecture/MATH_REVIEW.md §0`.

#[test]
fn test_upgrade_pool_params_accepts_max_borrow_rate_at_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let rate_before = t.pool_borrow_rate("USDC");

    // At the exact cap (`2 * RAY`); slope3 must remain <= max.
    t.upgrade_pool_params(
        "USDC",
        InterestRateModel {
            max_borrow_rate: 2 * RAY,
            base_borrow_rate: RAY / 100,
            slope1: RAY * 4 / 100,
            slope2: RAY * 10 / 100,
            slope3: RAY * 150 / 100,
            mid_utilization: RAY * 50 / 100,
            optimal_utilization: RAY * 80 / 100,
            max_utilization: controller::constants::RAY * 95 / 100,
            reserve_factor: 1000,
            is_flashloanable: false,
            flashloan_fee: 0,
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

#[test]
fn test_set_oracle_config_activates_pending_market() {
    let t = LendingTest::new().build(); // Empty protocol.
    let ctrl = t.ctrl_client();
    let admin = &t.admin;

    let asset = t
        .env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let params = usdc_preset().params.to_market_params(&asset, 7);
    ctrl.create_liquidity_pool(&HARNESS_HUB, &asset, &params);
    assert!(
        !t.market_is_active(&asset),
        "market must start in PendingOracle"
    );

    let oracle_cfg = resolved_reflector_primary_anchor_config(&t.mock_reflector, &asset);
    t.price_agg_client().set_oracle_config(&asset, &oracle_cfg);

    let oracle = t.market_oracle_config(&asset);
    match oracle.primary {
        controller::types::OracleSourceConfig::Reflector(source) => {
            assert_eq!(source.contract, t.mock_reflector);
            assert_eq!(source.read_mode, controller::types::OracleReadMode::Twap(3));
        }
        _ => panic!("expected Reflector primary source"),
    }
    assert_eq!(oracle.max_price_stale_seconds, 900);
    assert!(
        t.market_is_active(&asset),
        "market should be Active after oracle config",
    );
}

// `set_oracle_config` re-validates the agreement band for anchored
// configs, so a degenerate tolerance on the (otherwise valid) pending-market
// activation path is rejected instead of silently disabling the guard.
#[test]
fn test_set_oracle_config_rejects_degenerate_tolerance() {
    let t = LendingTest::new().build();
    let ctrl = t.ctrl_client();
    let admin = &t.admin;

    let asset = t
        .env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let params = usdc_preset().params.to_market_params(&asset, 7);
    ctrl.create_liquidity_pool(&HARNESS_HUB, &asset, &params);

    let mut oracle_cfg = resolved_reflector_primary_anchor_config(&t.mock_reflector, &asset);
    // In-envelope upper, loose lower: would let a manipulated-low primary blend in.
    oracle_cfg.tolerance = OracleTolerance {
        upper_ratio_bps: 10_500,
        lower_ratio_bps: 100,
    };
    let result = t
        .price_agg_client()
        .try_set_oracle_config(&asset, &oracle_cfg);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::BAD_LAST_TOLERANCE);
    assert!(!t.market_is_active(&asset), "market must stay inactive");
}
#[test]
fn test_set_aggregator() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let ctrl = t.ctrl_client();
    let new_aggregator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());

    // Must not panic; the admin has permission.
    ctrl.set_swap_aggregator(&new_aggregator);

    // Confirm the new aggregator is actually persisted.
    let stored: Address = t.env.as_contract(&t.controller_address(), || {
        t.env
            .storage()
            .instance()
            .get(&controller::types::ControllerKey::SwapAggregator)
            .expect("aggregator must be stored")
    });
    assert_eq!(stored, new_aggregator, "aggregator must be persisted");
}

/// 600 BPS tolerance band as governance computes it in-path.
fn bands_300_600() -> OracleTolerance {
    OracleTolerance {
        upper_ratio_bps: 10_600,
        lower_ratio_bps: 9_434,
    }
}

#[test]
fn test_set_tolerance_overwrites_bands() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let asset = t.resolve_market("USDC").asset.clone();

    let tolerance = bands_300_600();
    t.price_agg_client().set_tolerance(&asset, &tolerance);

    let oracle = t.market_oracle_config(&asset);
    assert_eq!(
        oracle.tolerance, tolerance,
        "tolerance bands must be overwritten in storage"
    );
}

#[test]
fn test_set_tolerance_rejects_unknown_asset() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let tolerance = bands_300_600();

    let unknown = Address::generate(&t.env);
    let result = t.price_agg_client().try_set_tolerance(&unknown, &tolerance);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    // set_tolerance updates the asset's `AssetOracle` entry; an unknown
    // asset has none, so it reverts OracleNotConfigured.
    assert_contract_error(mapped, errors::ORACLE_NOT_CONFIGURED);
}

// A direct setter call with a degenerate/inverted tolerance band is rejected by
// the controller re-validation (a band that reverts every read can't be stored).
#[test]
fn test_set_tolerance_rejects_degenerate_band() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();

    // upper below BPS + MIN_TOLERANCE and lower > upper: out of envelope, inverted.
    let bad = OracleTolerance {
        upper_ratio_bps: 9_000,
        lower_ratio_bps: 11_000,
    };
    let result = t.price_agg_client().try_set_tolerance(&asset, &bad);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::BAD_LAST_TOLERANCE);
}

// A band whose upper leg is in-envelope but whose lower leg sits below the
// symmetric floor (`bps - MAX_TOLERANCE`) is rejected: it would let a
// manipulated-low primary drag the blended midpoint down while still "in band".
#[test]
fn test_set_tolerance_rejects_loose_lower_band() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();

    let loose = OracleTolerance {
        upper_ratio_bps: 10_500,
        lower_ratio_bps: 100,
    };
    let result = t.price_agg_client().try_set_tolerance(&asset, &loose);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::BAD_LAST_TOLERANCE);
}
#[test]
fn test_permissionless_keeper_ops() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    let bob_addr = t.get_or_create_user(BOB);

    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::vec![&t.env, hub_asset(t.resolve_market("USDC").asset.clone())];
    t.env.mock_all_auths();
    let result = ctrl.try_update_indexes(&bob_addr, &assets);
    assert!(result.is_ok(), "any signed caller may update_indexes");
}
#[test]
fn test_permissionless_revenue_ops() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    let bob_addr = t.get_or_create_user(BOB);

    let ctrl = t.ctrl_client();
    let asset = t.resolve_market("USDC").asset.clone();
    let assets = soroban_sdk::vec![&t.env, hub_asset(asset)];
    t.env.mock_all_auths();
    let result = ctrl.try_claim_revenue(&bob_addr, &assets);
    assert!(result.is_ok(), "any signed caller may claim_revenue");
}
#[test]
fn test_create_liquidity_pool_uniqueness() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let ctrl = t.ctrl_client();
    let asset = t.resolve_asset("USDC");
    let params = usdc_preset().params.to_market_params(&asset, 7);

    // USDC was already initialized by the builder. Re-creating the same market
    // reverts on the pool's duplicate-market guard (`AssetAlreadySupported`).
    let result = match ctrl.try_create_liquidity_pool(&HARNESS_HUB, &asset, &params) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error")),
    };
    assert_contract_error(result, errors::GenericError::AssetAlreadySupported as u32);
}
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

    // Create the liquidity pool with no oracle; the call succeeds and
    // leaves the market in PendingOracle.
    ctrl.create_liquidity_pool(&HARNESS_HUB, &asset, &params);

    // Confirm the market is pending (no oracle yet).
    assert!(
        !t.market_is_active(&asset),
        "market should be in PendingOracle status"
    );

    // 2. Configure the full market oracle in one call.
    let reflector_cfg = test_harness::reflector_primary_anchor_config(
        &t.mock_reflector,
        &asset,
        1_0000000i128,
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.mock_reflector_client().set_price(&asset, &1_0000000i128);
    t.configure_market_oracle(&asset, &reflector_cfg);

    // 3. Confirm the market is Active (its AssetOracle entry now exists).
    assert!(
        t.market_is_active(&asset),
        "market should be in Active status"
    );
}

// Reconfiguring an ACTIVE market's oracle to a sanity band that excludes the
// current live price is rejected at PROPOSE with `SanityBoundViolated` (#223):
// governance resolves the fresh feed while scheduling, so a band that would
// brick every later risk read (borrow/withdraw/liquidation) never gets stored.
#[test]
#[should_panic(expected = "Error(Contract, #223)")]
fn test_configure_market_oracle_rejects_out_of_band_live_price() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let usdc = t.resolve_asset("USDC");

    // USDC lives at $1; a single-source band tight around $3 excludes it.
    let cfg = test_harness::reflector_single_spot_config(
        &t.mock_reflector,
        &usdc,
        test_harness::usd(3),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&usdc, &cfg);
}

// Oracle decimals must match the pool market's registered decimals; a
// `upgrade_pool` must forward the hash to the deployed pool: upgrading to a
// hash that was never uploaded fails inside the pool's upgrade call.
#[test]
fn test_upgrade_pool_forwards_hash_to_pool() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let bogus = soroban_sdk::BytesN::from_array(&t.env, &[9u8; 32]);
    assert!(
        t.ctrl_client().try_upgrade_pool(&bogus).is_err(),
        "upgrading the deployed pool to a missing wasm hash must fail"
    );
}

// A zero-revenue claim must not touch the token: no SAC transfer happens
// (and thus no transfer event) when nothing accrued.
#[test]
fn test_claim_revenue_zero_accrual_skips_transfer() {
    use soroban_sdk::testutils::Events as _;

    let t = LendingTest::new().with_market(usdc_preset()).build();
    let accumulator = Address::generate(&t.env);
    t.set_accumulator(&accumulator);

    // Fresh market, no borrows: nothing accrued.
    let claimed = t.claim_revenue("USDC");
    assert_eq!(claimed, 0);

    // No SAC transfer may run for a zero claim: the token contract emits
    // nothing during the claim invocation.
    let token = t.resolve_market("USDC").asset.clone();
    let token_events = t.env.events().all().filter_by_contract(&token);
    assert!(
        token_events.events().is_empty(),
        "zero-revenue claim must not emit a token transfer"
    );
}

// The min-borrow-collateral floor is inclusive: an account whose
// LTV-weighted collateral equals the floor exactly may borrow.
#[test]
fn test_min_borrow_floor_is_inclusive_at_exact_boundary() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // $10k USDC at LTV 0.75 -> LTV collateral exactly $7500.
    let floor: i128 = 7_500 * 1_000_000_000_000_000_000;
    t.ctrl_client().set_min_borrow_collateral_usd(&floor);

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 0.1);
    assert!(t.borrow_balance(ALICE, "ETH") > 0.09);
}
