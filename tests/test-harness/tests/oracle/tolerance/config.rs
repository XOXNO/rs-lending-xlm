use super::{enable_dual_source, setup};
use test_harness::{eth_preset, usdc_preset, LendingTest, ALICE, HARNESS_HUB};

#[test]
fn test_tolerance_config_valid_update() {
    let t = setup();

    let asset = t.resolve_market("USDC").asset.clone();

    let tolerance = controller::types::OracleTolerance {
        upper_ratio_bps: 10_600,
        lower_ratio_bps: 9_434,
    };
    let result = t.price_agg_client().try_set_tolerance(
        &controller::types::PriceKey::Token(asset.clone()),
        &tolerance,
    );
    assert!(result.is_ok(), "valid tolerance update should succeed");
}

#[test]
fn test_tolerance_config_rejects_non_reciprocal_lower() {
    let t = setup();
    let asset = t.resolve_market("USDC").asset.clone();

    let result = t.price_agg_client().try_set_tolerance(
        &controller::types::PriceKey::Token(asset),
        &controller::types::OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_500,
        },
    );
    assert!(
        result.is_err(),
        "non-reciprocal tolerance must fail BadLastTolerance"
    );
}

#[test]
fn test_dual_source_prices_and_risk_gates_still_resolve() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    let hf = t.health_factor(ALICE);
    assert!(hf > 0.0, "dual-source portfolio must still hard-price");
}

#[test]
fn test_set_accumulator() {
    let t = setup();
    let ctrl = t.ctrl_client();

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());

    ctrl.set_accumulator(&accumulator);

    let stored: soroban_sdk::Address = t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .instance()
            .get(&controller::types::ControllerKey::Accumulator)
            .unwrap()
    });
    assert_eq!(stored, accumulator, "accumulator address should be stored");
}

#[test]
fn test_edit_asset_in_spoke_category() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_spoke(2, test_harness::STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_dust_disabled_all_markets()
        .build();

    t.edit_asset_in_spoke("USDC", 2, true, false, 9700, 9800, 200);

    let usdc_asset = t.resolve_market("USDC").asset.clone();
    let config: Option<controller::types::SpokeAssetConfig> =
        t.env.as_contract(&t.controller, || {
            t.env
                .storage()
                .persistent()
                .get(&controller::types::ControllerKey::SpokeAsset(
                    2,
                    controller::types::HubAssetKey {
                        hub_id: HARNESS_HUB,
                        asset: usdc_asset,
                    },
                ))
        });
    let config = config.expect("spoke asset config should exist");
    assert!(
        config.is_collateralizable,
        "should still be collateralizable"
    );
    assert!(
        !config.is_borrowable,
        "should no longer be borrowable after edit"
    );
}
