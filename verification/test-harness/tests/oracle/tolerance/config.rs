use super::setup;
use test_harness::{eth_preset, usdc_preset, LendingTest, ALICE};

// 6. Oracle tolerance config validation (controller side)

#[test]
fn test_tolerance_config_rejects_first_below_min() {
    let t = setup();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let asset = t.resolve_market("USDC").asset.clone();

    // MIN_FIRST_TOLERANCE = 50 BPS.
    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &10, &500);
    assert!(
        result.is_err(),
        "first tolerance below 50 BPS should be rejected"
    );
}

#[test]
fn test_tolerance_config_rejects_first_above_max() {
    let t = setup();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let asset = t.resolve_market("USDC").asset.clone();

    // MAX_FIRST_TOLERANCE = 5000 BPS.
    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &6000, &7000);
    assert!(
        result.is_err(),
        "first tolerance above 5000 BPS should be rejected"
    );
}

#[test]
fn test_tolerance_config_rejects_last_below_min() {
    let t = setup();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let asset = t.resolve_market("USDC").asset.clone();

    // MIN_LAST_TOLERANCE = 150 BPS, first=200 is valid.
    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &100, &100);
    assert!(
        result.is_err(),
        "last tolerance below 150 BPS should be rejected"
    );
}

#[test]
fn test_tolerance_config_rejects_last_above_max() {
    let t = setup();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let asset = t.resolve_market("USDC").asset.clone();

    // MAX_LAST_TOLERANCE = 10000 BPS.
    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &200, &11000);
    assert!(
        result.is_err(),
        "last tolerance above 10000 BPS should be rejected"
    );
}

#[test]
fn test_tolerance_config_rejects_last_less_than_first() {
    let t = setup();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let asset = t.resolve_market("USDC").asset.clone();

    // last (200) < first (300): must fail.
    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &300, &200);
    assert!(
        result.is_err(),
        "last tolerance < first tolerance should be rejected"
    );
}

#[test]
fn test_tolerance_config_valid_update() {
    let t = setup();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let asset = t.resolve_market("USDC").asset.clone();

    // Valid tolerance update.
    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &300, &600);
    assert!(result.is_ok(), "valid tolerance update should succeed");
}
// 7. Config gap tests

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
fn test_set_liquidity_pool_template() {
    let t = setup();
    let ctrl = t.ctrl_client();

    let hash = soroban_sdk::BytesN::from_array(&t.env, &[42u8; 32]);

    ctrl.set_liquidity_pool_template(&hash);

    // Verify storage by reading directly.
    let stored: soroban_sdk::BytesN<32> = t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .instance()
            .get(&controller::types::ControllerKey::PoolTemplate)
            .unwrap()
    });
    assert_eq!(stored, hash, "pool template hash should be stored");
}

#[test]
fn test_disable_token_oracle_blocks_operations() {
    let mut t = setup();

    t.supply(ALICE, "USDC", 10_000.0);

    // Disable the USDC oracle: oracle_type becomes 0 (None).
    let usdc_asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    t.ctrl_client().disable_token_oracle(&admin, &usdc_asset);

    // The disabled USDC oracle returns zero, changing HF-sensitive behavior.
    // Borrowing against zero-value collateral must fail.
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert!(
        result.is_err(),
        "borrow should fail when collateral oracle is disabled (price=0)"
    );
}

#[test]
fn test_edit_asset_in_e_mode_category() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_emode(1, test_harness::STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_dust_disabled_all_markets()
        .build();

    // Initially: can_collateral=true, can_borrow=true.
    // Edit: set can_borrow=false.
    t.edit_asset_in_e_mode("USDC", 1, true, false);

    // Verify the update by reading storage.
    let usdc_asset = t.resolve_market("USDC").asset.clone();
    let config: Option<controller::types::EModeAssetConfig> = t.env.as_contract(&t.controller, || {
        let cat: Option<controller::types::EModeCategoryRaw> = t
            .env
            .storage()
            .persistent()
            .get(&controller::types::ControllerKey::EModeCategory(1));
        cat.and_then(|c| c.assets.get(usdc_asset))
    });
    let config = config.expect("emode asset config should exist");
    assert!(
        config.is_collateralizable,
        "should still be collateralizable"
    );
    assert!(
        !config.is_borrowable,
        "should no longer be borrowable after edit"
    );
}
