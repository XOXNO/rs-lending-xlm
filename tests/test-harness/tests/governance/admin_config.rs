//! Risk-bound, position-limit, IRM-cap, and tolerance validation on the
//! governance forwarders.

use controller::constants::RAY;
use controller::types::InterestRateModel;
use governance::op::{AdminOperation, SpokeAssetArgs, UpgradePoolParamsArgs};
use test_harness::{
    assert_contract_error, errors, hub_asset, usdc_preset, LendingTest, HARNESS_HUB, HARNESS_SPOKE,
};

// `validate_risk_bounds` rejects threshold == LTV (#113).
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
        threshold: 8000, // Equal to LTV.
        bonus: config.liquidation_bonus,
        liquidation_fees: config.liquidation_fees,
        supply_cap: config.supply_cap,
        borrow_cap: config.borrow_cap,
        oracle_override: config.oracle_override,
    };

    let result = gov.try_execute_immediate(&admin, &AdminOperation::EditAssetInSpoke(args));
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::INVALID_LIQ_THRESHOLD);
}

// Boundary regression for the Slender C-3 / Blend BL-001 resource-limit DoS
// class (see audit-research/STELLAR_AUDIT_FINDINGS.md §4.4). The hard cap on
// per-account positions must match the budget bench at
// `bench_liquidate_max_positions.rs`; raising it without re-running the bench
// re-introduces the un-liquidatable-position attack surface.
#[test]
fn test_set_position_limits_rejects_above_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let admin = t.admin();

    // 10/10 is the documented ceiling and must be accepted.
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::SetPositionLimits(controller::types::PositionLimits {
            max_supply_positions: 10,
            max_borrow_positions: 10,
        }),
    );

    // 11 on either side exceeds the budget-proven envelope.
    assert_invalid_position_limits(&t, 11, 10);
    assert_invalid_position_limits(&t, 10, 11);
    // The previous-cap value (32) must also be rejected post-fix.
    assert_invalid_position_limits(&t, 32, 32);
    // Zero on either side stays rejected (existing invariant).
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

// Regression: `max_borrow_rate` cap (Taylor envelope).
//
// `InterestRateModel::verify` (run by governance before forwarding, and again
// by `pool::update_params`) rejects any `max_borrow_rate > 2 * RAY` to
// keep `compound_interest`'s 8-term Taylor approximation inside its
// documented `< 0.01 %` accuracy envelope. See `architecture/MATH_REVIEW.md §0`.
#[test]
fn test_upgrade_pool_params_rejects_max_borrow_rate_above_cap() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let gov = t.gov_client();
    let admin = t.admin();

    // `2 * RAY + 1` exceeds MAX_BORROW_RATE_RAY → MAX_BORROW_RATE_TOO_HIGH (#131).
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
            },
        }),
    );
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::MAX_BORROW_RATE_TOO_HIGH);
}
