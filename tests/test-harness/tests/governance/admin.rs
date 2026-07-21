//! Token-shape probing and asset-config bounds on the governance forwarders.

use governance::op::{AdminOperation, ConfigureOracleArgs, CreatePoolArgs, SpokeAssetArgs};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_harness::{
    assert_contract_error, errors, hub_asset, usdc_preset, LendingTest, HARNESS_HUB, HARNESS_SPOKE,
};

// `validate_and_fetch_token_decimals` rejects SACs without a `symbol` (#6).
#[test]
fn test_create_liquidity_pool_rejects_token_without_symbol() {
    let t = LendingTest::new().build();
    let admin = t.admin();
    let gov = t.gov_client();
    let sac = t.env.register(test_harness::mock_sac::MockSacNoSymbol, ());
    let params = usdc_preset().params.to_market_params(&sac, 7);
    let result = match gov.try_execute_immediate(
        &admin,
        &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
            hub_id: HARNESS_HUB,
            asset: sac.clone(),
            params,
        }),
    ) {
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
    let result = match gov.try_execute_immediate(
        &admin,
        &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
            hub_id: HARNESS_HUB,
            asset: asset.clone(),
            params,
        }),
    ) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error")),
    };
    assert_contract_error(result, errors::INVALID_ASSET);
}

// `validate_market_creation` rejects params.asset_decimals != live SAC decimals (#6).
#[test]
fn test_create_liquidity_pool_rejects_asset_decimals_mismatch() {
    use soroban_sdk::token;

    let t = LendingTest::new().build();
    let admin = t.admin();
    let gov = t.gov_client();
    let asset = t
        .env
        .register_stellar_asset_contract_v2(admin.clone())
        .address()
        .clone();
    let decimals = token::Client::new(&t.env, &asset).decimals();
    let mismatched = decimals.saturating_add(1);
    let params = usdc_preset().params.to_market_params(&asset, mismatched);

    let result = match gov.try_execute_immediate(
        &admin,
        &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
            hub_id: HARNESS_HUB,
            asset: asset.clone(),
            params,
        }),
    ) {
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
    t.gov_client()
        .execute_immediate(&admin, &AdminOperation::SetMinBorrowCollateralUsd(-1));
}

// `validate_risk_bounds` threshold above 100% (#113).
#[test]
#[should_panic(expected = "Error(Contract, #113)")]
fn test_edit_asset_in_spoke_rejects_threshold_above_bps() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let admin = t.admin();
    let asset = t.resolve_market("USDC").asset.clone();
    let cfg = t
        .ctrl_client()
        .get_spoke_asset(&HARNESS_SPOKE, &hub_asset(asset.clone()));
    let args = SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset,
        spoke_id: HARNESS_SPOKE,
        can_collateral: cfg.is_collateralizable,
        can_borrow: cfg.is_borrowable,
        paused: false,
        frozen: false,
        ltv: 5_000,
        threshold: 10_001,
        bonus: 0,
        liquidation_fees: cfg.liquidation_fees,
        supply_cap: cfg.supply_cap,
        borrow_cap: cfg.borrow_cap,
    };
    t.gov_client()
        .execute_immediate(&admin, &AdminOperation::EditAssetInSpoke(args));
}

// Configure-time tolerance below the minimum (#208).
#[test]
#[should_panic(expected = "Error(Contract, #208)")]
fn test_configure_market_oracle_rejects_tolerance_below_min() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    let cfg = test_harness::reflector_primary_anchor_config(&t.mock_reflector, &asset, 10);
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::ConfigureMarketOracle(ConfigureOracleArgs {
            hub_asset: hub_asset(asset),
            cfg,
        }),
    );
}
