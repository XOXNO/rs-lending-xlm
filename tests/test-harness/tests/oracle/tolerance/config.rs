use super::setup;
use test_harness::{eth_preset, usdc_preset, LendingTest, HARNESS_HUB};

#[test]
fn test_tolerance_config_valid_update() {
    let t = setup();

    let asset = t.resolve_market("USDC").asset.clone();

    // 600 BPS band as governance computes it in-path.
    let tolerance = controller::types::OracleTolerance {
        upper_ratio_bps: 10_600,
        lower_ratio_bps: 9_434,
    };
    let result = t.price_agg_client().try_set_tolerance(&asset, &tolerance);
    assert!(result.is_ok(), "valid tolerance update should succeed");
}

#[test]
fn test_set_accumulator() {
    let t = setup();
    let ctrl = t.ctrl_client();

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());

    // Must not panic: admin has permission.
    ctrl.set_accumulator(&accumulator);

    // Verify storage by reading directly.
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

    // Initially: can_collateral=true, can_borrow=true.
    // Edit: set can_borrow=false.
    t.edit_asset_in_spoke("USDC", 2, true, false, 9700, 9800, 200);

    // Verify the update by reading storage. Spoke asset configs are discrete
    // `SpokeAsset(spoke_id, hub_asset)` keys in the spoke model.
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
