//! Token-shape probing and asset-config bounds on the governance forwarders.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_harness::{assert_contract_error, errors, usdc_preset, LendingTest, DEFAULT_TOLERANCE};
use governance::op::{AdminOperation, CreatePoolArgs, ConfigureOracleArgs};

// `validate_and_fetch_token_decimals` rejects SACs without a `symbol` (#6).
#[test]
fn test_create_liquidity_pool_rejects_token_without_symbol() {
    let t = LendingTest::new().build();
    let admin = t.admin();
    let gov = t.gov_client();
    let sac = t.env.register(test_harness::mock_sac::MockSacNoSymbol, ());
    let params = usdc_preset().params.to_market_params(&sac, 7);
    let config = usdc_preset().config.to_asset_config(&t.env, 7);
    gov.execute_immediate(&admin, &AdminOperation::ApproveToken(sac.clone()));
    let result = match gov.try_execute_immediate(&admin, &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
        asset: sac.clone(),
        params,
        config,
    })) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error")),
    };
    assert_contract_error(result, errors::INVALID_ASSET);
}

// `validate_and_fetch_token_decimals` rejects unregistered token contracts (#6).
#[test]
fn test_create_liquidity_pool_rejects_unregistered_token() {
    let t = LendingTest::new().build();
    let admin = t.admin();
    let gov = t.gov_client();
    let asset = Address::generate(&t.env);
    let params = usdc_preset().params.to_market_params(&asset, 7);
    let config = usdc_preset().config.to_asset_config(&t.env, 7);
    gov.execute_immediate(&admin, &AdminOperation::ApproveToken(asset.clone()));
    let result = match gov.try_execute_immediate(&admin, &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
        asset: asset.clone(),
        params,
        config,
    })) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error")),
    };
    assert_contract_error(result, errors::INVALID_ASSET);
}

// `set_min_borrow_collateral_usd` rejects negative floors (#116).
#[test]
#[should_panic(expected = "Error(Contract, #116)")]
fn test_set_min_borrow_collateral_rejects_negative_floor() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let admin = t.admin();
    t.gov_client().execute_immediate(&admin, &AdminOperation::SetMinBorrowCollateralUsd(-1));
}

// `validate_risk_bounds` threshold above 100% (#113).
#[test]
#[should_panic(expected = "Error(Contract, #113)")]
fn test_edit_asset_config_rejects_threshold_above_bps() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let admin = t.admin();
    let asset = t.resolve_market("USDC").asset.clone();
    let mut cfg = t.ctrl_client().get_market_config(&asset).asset_config;
    cfg.loan_to_value_bps = 5_000;
    cfg.liquidation_threshold_bps = 10_001;
    cfg.liquidation_bonus_bps = 0;
    t.gov_client().execute_immediate(&admin, &AdminOperation::EditAssetConfig(asset, cfg));
}

// Configure-time bad first tolerance (#207).
#[test]
#[should_panic(expected = "Error(Contract, #207)")]
fn test_configure_market_oracle_rejects_first_tolerance_below_min() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let cfg = test_harness::reflector_primary_anchor_config(
        &t.mock_reflector,
        &asset,
        10,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.gov_client().execute_immediate(&admin, &AdminOperation::ConfigureMarketOracle(ConfigureOracleArgs {
        asset,
        cfg,
    }));
}
