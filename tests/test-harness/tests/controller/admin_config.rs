use controller::constants::RAY;
use controller::types::{
    AssetOracle, FeedSource, IndependencePolicy, InterestRateModel, OracleAssetRef, OracleReadMode,
    OracleTolerance, PositionLimits, PriceSource, ProviderRef, ReflectorFeedRef,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_harness::{
    assert_contract_error, errors, hub_asset, map_try_ok_unit, usdc_preset, LendingTest, ALICE,
    BOB, DEFAULT_TOLERANCE, HARNESS_HUB,
};

fn resolved_reflector_dual_source_config(
    env: &soroban_sdk::Env,
    oracle: &Address,
    asset: &Address,
) -> AssetOracle {
    let source = |read_mode: OracleReadMode| {
        PriceSource::Feed(FeedSource {
            provider: ProviderRef::Reflector(ReflectorFeedRef {
                contract: oracle.clone(),
                asset: OracleAssetRef::Stellar(asset.clone()),
                read_mode,
            }),
            decimals: 14,
            max_stale_seconds: 900,
        })
    };
    let mut sources = soroban_sdk::Vec::new(env);
    sources.push_back(source(OracleReadMode::Twap(3)));
    sources.push_back(source(OracleReadMode::Spot));
    AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },

        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: 1,
        max_sanity_price_wad: controller::constants::MAX_REASONABLE_PRICE_WAD,
    }
}
#[test]
fn test_edit_asset_config() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 6000;
    });

    let config = t.get_asset_config("USDC");
    assert_eq!(config.loan_to_value, 6000, "LTV should be updated to 6000");

    assert_eq!(
        config.liquidation_threshold, 8000,
        "threshold should remain 8000"
    );
}
#[test]
fn test_set_position_limits() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    t.set_position_limits(4, 3);

    let limits = t.get_position_limits();
    assert_eq!(limits.max_supply_positions, 4);
    assert_eq!(limits.max_borrow_positions, 3);
}

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
        let mapped = map_try_ok_unit(result);
        assert_contract_error(mapped, errors::GenericError::InvalidPositionLimits as u32);
    }
}

#[test]
fn test_pause_blocks_operations() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);

    t.pause();

    let supply_result = t.try_supply(ALICE, "USDC", 1000.0);
    assert_contract_error(supply_result, errors::CONTRACT_PAUSED);

    let borrow_result = t.try_borrow(ALICE, "ETH", 0.5);
    assert_contract_error(borrow_result, errors::CONTRACT_PAUSED);
}
#[test]
fn test_unpause_restores_operations() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);

    t.pause();

    let result = t.try_supply(ALICE, "USDC", 1000.0);
    assert_contract_error(result, errors::CONTRACT_PAUSED);

    t.unpause();

    let result = t.try_supply(ALICE, "USDC", 1000.0);
    assert!(result.is_ok(), "supply should work after unpause");
}
#[test]
fn test_upgrade_pool_params() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let rate_before = t.pool_borrow_rate("USDC");

    let new_base_rate = RAY * 2 / 100;
    let new_slope1 = RAY * 8 / 100;

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

#[test]
fn test_upgrade_pool_params_accepts_max_borrow_rate_at_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let rate_before = t.pool_borrow_rate("USDC");

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

    let _ = rate_before;
    // The at-cap model must be accepted AND stored: at zero utilization the
    // borrow rate equals the new base rate (RAY/100 = 1%).
    let rate_after = t.pool_borrow_rate("USDC");
    assert!(
        (rate_after - 0.01).abs() < 1e-9,
        "stored curve must serve the new 1% base at zero utilization, got {rate_after}",
    );
}

#[test]
fn test_seeding_an_oracle_activates_a_pending_market() {
    let t = LendingTest::new().build();
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

    let oracle_cfg = resolved_reflector_dual_source_config(&t.env, &t.mock_reflector, &asset);
    t.price_agg_client().seed_oracle(
        &controller::types::PriceKey::Token(asset.clone()),
        &oracle_cfg,
    );

    let oracle = t.market_oracle_config(&asset);
    match oracle.sources.get_unchecked(0) {
        controller::types::PriceSource::Feed(feed) => match feed.provider {
            ProviderRef::Reflector(source) => {
                assert_eq!(source.contract, t.mock_reflector);
                assert_eq!(source.read_mode, OracleReadMode::Twap(3));
            }
            _ => panic!("expected Reflector provider"),
        },
        _ => panic!("expected a direct feed source"),
    }
    assert_eq!(oracle.max_price_stale_seconds, 900);
    assert!(
        t.market_is_active(&asset),
        "market should be Active after oracle config",
    );
}

#[test]
fn test_set_oracle_rejects_a_degenerate_tolerance() {
    let t = LendingTest::new().build();
    let ctrl = t.ctrl_client();
    let admin = &t.admin;

    let asset = t
        .env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let params = usdc_preset().params.to_market_params(&asset, 7);
    ctrl.create_liquidity_pool(&HARNESS_HUB, &asset, &params);

    let mut oracle_cfg = resolved_reflector_dual_source_config(&t.env, &t.mock_reflector, &asset);

    oracle_cfg.min_sanity_price_wad = 990_000_000_000_000_000;
    oracle_cfg.max_sanity_price_wad = 1_010_000_000_000_000_000;
    oracle_cfg.tolerance = OracleTolerance {
        upper_ratio_bps: 10_500,
        lower_ratio_bps: 100,
    };
    let result = t.price_agg_client().try_set_oracle(
        &controller::types::PriceKey::Token(asset.clone()),
        &oracle_cfg,
    );
    let mapped = map_try_ok_unit(result);
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

    ctrl.set_swap_aggregator(&new_aggregator);

    let stored: Address = t.env.as_contract(&t.controller_address(), || {
        t.env
            .storage()
            .instance()
            .get(&controller::types::ControllerKey::SwapAggregator)
            .expect("aggregator must be stored")
    });
    assert_eq!(stored, new_aggregator, "aggregator must be persisted");
}

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
    t.price_agg_client().set_tolerance(
        &controller::types::PriceKey::Token(asset.clone()),
        &tolerance,
    );

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
    let result = t.price_agg_client().try_set_tolerance(
        &controller::types::PriceKey::Token(unknown.clone()),
        &tolerance,
    );
    let mapped = map_try_ok_unit(result);

    assert_contract_error(mapped, errors::ORACLE_NOT_CONFIGURED);
}

#[test]
fn test_set_tolerance_rejects_degenerate_band() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();

    let bad = OracleTolerance {
        upper_ratio_bps: 9_000,
        lower_ratio_bps: 11_000,
    };
    let result = t
        .price_agg_client()
        .try_set_tolerance(&controller::types::PriceKey::Token(asset.clone()), &bad);
    let mapped = map_try_ok_unit(result);
    assert_contract_error(mapped, errors::BAD_LAST_TOLERANCE);
}

#[test]
fn test_set_tolerance_rejects_loose_lower_band() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();

    let loose = OracleTolerance {
        upper_ratio_bps: 10_500,
        lower_ratio_bps: 100,
    };
    let result = t
        .price_agg_client()
        .try_set_tolerance(&controller::types::PriceKey::Token(asset.clone()), &loose);
    let mapped = map_try_ok_unit(result);
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

    let result = match ctrl.try_create_liquidity_pool(&HARNESS_HUB, &asset, &params) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error")),
    };
    assert_contract_error(result, errors::GenericError::AssetAlreadySupported as u32);
}
#[test]
fn test_market_initialization_cascade() {
    let t = LendingTest::new().build();
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
        "market should be in PendingOracle status"
    );

    let reflector_cfg = test_harness::reflector_primary_anchor_config(
        &t.env,
        &t.mock_reflector,
        &asset,
        1_0000000i128,
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.mock_reflector_client().set_price(&asset, &1_0000000i128);
    t.configure_market_oracle(&asset, &reflector_cfg);

    assert!(
        t.market_is_active(&asset),
        "market should be in Active status"
    );
}

#[test]
fn test_configure_market_oracle_defers_an_out_of_band_price_to_read_time() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let usdc = t.resolve_asset("USDC");

    let cfg = test_harness::reflector_primary_anchor_config(
        &t.env,
        &t.mock_reflector,
        &usdc,
        test_harness::usd(3),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&usdc, &cfg);

    let result = t.price_agg_client().try_prices(&soroban_sdk::vec![
        &t.env,
        controller::types::PriceKey::Token(usdc.clone())
    ]);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::SANITY_BOUND_VIOLATED);
}

#[test]
fn test_upgrade_pool_forwards_hash_to_pool() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let bogus = soroban_sdk::BytesN::from_array(&t.env, &[9u8; 32]);
    assert!(
        t.ctrl_client().try_upgrade_pool(&bogus).is_err(),
        "upgrading the deployed pool to a missing wasm hash must fail"
    );
}

#[test]
fn test_claim_revenue_zero_accrual_skips_transfer() {
    use soroban_sdk::testutils::Events as _;

    let t = LendingTest::new().with_market(usdc_preset()).build();
    let accumulator = Address::generate(&t.env);
    t.set_accumulator(&accumulator);

    let claimed = t.claim_revenue("USDC");
    assert_eq!(claimed, 0);

    let token = t.resolve_market("USDC").asset.clone();
    let token_events = t.env.events().all().filter_by_contract(&token);
    assert!(
        token_events.events().is_empty(),
        "zero-revenue claim must not emit a token transfer"
    );
}

#[test]
fn test_min_borrow_floor_is_inclusive_at_exact_boundary() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let floor: i128 = 7_500 * 1_000_000_000_000_000_000;
    t.ctrl_client().set_min_borrow_collateral_usd(&floor);

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 0.1);
    assert!(t.borrow_balance(ALICE, "ETH") > 0.09);
}
