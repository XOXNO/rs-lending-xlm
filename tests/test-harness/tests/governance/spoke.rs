//! Spoke risk-bound validation on the governance forwarder.
//!
//! Risk parameters are per-asset, so bound validation happens when an asset
//! joins a category, not at category creation.

use governance::op::{AdminOperation, SpokeAssetArgs};
use soroban_sdk::TryFromVal;
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, usdc_preset, LendingTest, HARNESS_HUB,
};

fn add_category(t: &LendingTest) -> u32 {
    let admin = t.admin();
    let val = t
        .gov_client()
        .execute_immediate(&admin, &AdminOperation::AddSpoke);
    u32::try_from_val(&t.env, &val).unwrap()
}

fn try_add_asset(
    t: &LendingTest,
    asset: &soroban_sdk::Address,
    category_id: u32,
    ltv: u32,
    threshold: u32,
    bonus: u32,
) -> Result<(), soroban_sdk::Error> {
    let admin = t.admin();
    let args = SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: asset.clone(),
        spoke_id: category_id,
        can_collateral: true,
        can_borrow: true,
        ltv,
        threshold,
        bonus,
        supply_cap: 0,
        borrow_cap: 0,
    };
    match t
        .gov_client()
        .try_execute_immediate(&admin, &AdminOperation::AddAssetToSpoke(args))
    {
        Ok(res) => res.map(|_| ()).map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

// threshold (8000) <= ltv (9000) must reject with InvalidLiqThreshold (113).
#[test]
fn test_spoke_rejects_threshold_lte_ltv() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let id = add_category(&t);
    let usdc = t.resolve_asset("USDC");
    let result = try_add_asset(&t, &usdc, id, 9000, 8000, 200);
    assert_contract_error(result, errors::INVALID_LIQ_THRESHOLD);
}

#[test]
fn test_spoke_accepts_valid_asset_bounds() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let id = add_category(&t);
    assert!(id > 0, "governance forwarder should return a category id");
    let usdc = t.resolve_asset("USDC");
    try_add_asset(&t, &usdc, id, 8000, 8500, 200).expect("valid asset should be accepted");

    let cfg = t
        .ctrl_client()
        .get_spoke_asset(&id, &hub_asset(usdc.clone()));
    assert_eq!(cfg.loan_to_value, 8000);
    assert_eq!(cfg.liquidation_threshold, 8500);
    assert_eq!(cfg.liquidation_bonus, 200);
}

#[test]
fn test_spoke_add_asset_via_gov_forwarder() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let id = add_category(&t);
    let usdc = t.resolve_asset("USDC");
    let admin = t.admin();
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
            liquidation_fees: 0,
            oracle_override: controller::types::MarketOracleConfigOption::None,
            hub_id: HARNESS_HUB,
            asset: usdc.clone(),
            spoke_id: id,
            can_collateral: true,
            can_borrow: true,
            ltv: 8000,
            threshold: 8500,
            bonus: 200,
            supply_cap: 0,
            borrow_cap: 0,
        }),
    );

    assert!(
        t.ctrl_client()
            .try_get_spoke_asset(&id, &hub_asset(usdc.clone()))
            .is_ok(),
        "USDC must be registered in the forwarded spoke"
    );
}

const UNIT: i128 = 10_000_000;

// Spoke caps are the only cap layer: a spoke cap of any size is accepted at
// add time (no hub-cap coupling) and enforced against spoke usage.
#[test]
fn test_spoke_accepts_spoke_caps_without_hub_coupling() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let id = add_category(&t);
    let usdc = t.resolve_asset("USDC");
    let admin = t.admin();

    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
            liquidation_fees: 0,
            oracle_override: controller::types::MarketOracleConfigOption::None,
            hub_id: HARNESS_HUB,
            asset: usdc.clone(),
            spoke_id: id,
            can_collateral: true,
            can_borrow: true,
            ltv: 8_000,
            threshold: 8_500,
            bonus: 200,
            supply_cap: 2_000 * UNIT,
            borrow_cap: 0,
        }),
    );

    let cfg = t
        .ctrl_client()
        .get_spoke_asset(&id, &hub_asset(usdc.clone()));
    assert_eq!(cfg.supply_cap, 2_000 * UNIT);
}
