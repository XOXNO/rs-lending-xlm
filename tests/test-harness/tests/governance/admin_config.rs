use controller::constants::RAY;
use controller::types::InterestRateModel;
use governance::op::{
    AdminOperation, SpokeAssetArgs, SpokeLiquidationCurveArgs, UpgradePoolParamsArgs,
};
use test_harness::{
    assert_contract_error, errors, hub_asset, usdc_preset, LendingTest, HARNESS_HUB, HARNESS_SPOKE,
};

#[test]
fn test_edit_asset_in_spoke_rejects_threshold_lte_ltv() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let admin = t.admin();
    let asset = t.resolve_market("USDC").asset.clone();
    let gov = t.gov_client();

    let config = t
        .ctrl_client()
        .get_spoke_asset(&HARNESS_SPOKE, &hub_asset(asset.clone()));
    let args = SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset,
        spoke_id: HARNESS_SPOKE,
        can_collateral: config.is_collateralizable,
        can_borrow: config.is_borrowable,
        paused: false,
        frozen: false,
        ltv: 8000,
        threshold: 8000,
        bonus: config.liquidation_bonus,
        liquidation_fees: config.liquidation_fees,
        supply_cap: config.supply_cap,
        borrow_cap: config.borrow_cap,
    };

    let result = gov.try_execute_immediate(&admin, &AdminOperation::EditAssetInSpoke(args));
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::INVALID_LIQ_THRESHOLD);
}

#[test]
fn test_set_position_limits_rejects_above_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let admin = t.admin();

    // Derived from the constant so a cap change is caught here, not by
    // unrelated fixtures downstream.
    let cap = common::constants::POSITION_LIMIT_MAX;
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::SetPositionLimits(controller::types::PositionLimits {
            max_supply_positions: cap,
            max_borrow_positions: cap,
        }),
    );

    assert_invalid_position_limits(&t, cap + 1, cap);
    assert_invalid_position_limits(&t, cap, cap + 1);

    assert_invalid_position_limits(&t, 32, 32);
    assert_invalid_position_limits(&t, 0, 5);
    assert_invalid_position_limits(&t, 5, 0);
}

fn assert_invalid_position_limits(t: &LendingTest, supply: u32, borrow: u32) {
    let admin = t.admin();
    let limits = controller::types::PositionLimits {
        max_supply_positions: supply,
        max_borrow_positions: borrow,
    };
    let result = t
        .gov_client()
        .try_execute_immediate(&admin, &AdminOperation::SetPositionLimits(limits));
    let expected = soroban_sdk::Error::from_contract_error(errors::INVALID_POSITION_LIMITS);
    match result {
        Ok(_) => panic!(
            "set_position_limits({}, {}) should have been rejected",
            supply, borrow
        ),
        Err(Ok(err)) => assert_eq!(
            err, expected,
            "set_position_limits({}, {}): expected INVALID_POSITION_LIMITS, got {:?}",
            supply, borrow, err
        ),
        Err(Err(invoke_err)) => panic!(
            "set_position_limits({}, {}) failed with host error {:?}",
            supply, borrow, invoke_err
        ),
    }
}

#[test]
fn test_set_spoke_liquidation_curve_overrides_defaults_end_to_end() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let admin = t.admin();

    let before = t.ctrl_client().get_spoke(&HARNESS_SPOKE);
    assert_eq!(before.liquidation_target_hf_wad, 1_100_000_000_000_000_000);

    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
            spoke_id: HARNESS_SPOKE,
            target_hf_wad: 1_010_000_000_000_000_000,
            hf_for_max_bonus_wad: 990_000_000_000_000_000,
            liquidation_bonus_factor_bps: 8_000,
        }),
    );

    let after = t.ctrl_client().get_spoke(&HARNESS_SPOKE);
    assert_eq!(after.liquidation_target_hf_wad, 1_010_000_000_000_000_000);
    assert_eq!(after.hf_for_max_bonus_wad, 990_000_000_000_000_000);
    assert_eq!(after.liquidation_bonus_factor_bps, 8_000);
}

#[test]
fn test_set_spoke_liquidation_curve_rejects_bonus_factor_above_bps() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let admin = t.admin();

    let result = t.gov_client().try_execute_immediate(
        &admin,
        &AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
            spoke_id: HARNESS_SPOKE,
            target_hf_wad: 1_020_000_000_000_000_000,
            hf_for_max_bonus_wad: 510_000_000_000_000_000,
            liquidation_bonus_factor_bps: 10_001,
        }),
    );
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::INVALID_LIQUIDATION_CURVE);
}

#[test]
fn test_set_spoke_liquidation_curve_rejects_unknown_spoke() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let admin = t.admin();

    let result = t.gov_client().try_execute_immediate(
        &admin,
        &AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
            spoke_id: 999,
            target_hf_wad: 1_020_000_000_000_000_000,
            hf_for_max_bonus_wad: 510_000_000_000_000_000,
            liquidation_bonus_factor_bps: 10_000,
        }),
    );
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::SPOKE_NOT_FOUND);
}

#[test]
fn test_upgrade_pool_params_rejects_max_borrow_rate_above_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let gov = t.gov_client();
    let admin = t.admin();

    let result = gov.try_execute_immediate(
        &admin,
        &AdminOperation::UpgradeLiquidityPoolParams(UpgradePoolParamsArgs {
            hub_asset: hub_asset(asset),
            params: InterestRateModel {
                max_borrow_rate: 2 * RAY + 1,
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
        }),
    );
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::MAX_BORROW_RATE_TOO_HIGH);
}
