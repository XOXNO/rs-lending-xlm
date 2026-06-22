//! E-mode risk-bound validation on the governance forwarder.
//!
//! Risk parameters are per-asset, so bound validation happens when an asset
//! joins a category, not at category creation.

use soroban_sdk::{BytesN, Env};
use test_harness::{assert_contract_error, errors, eth_preset, usdc_preset, LendingTest};

fn salt(env: &Env, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(env, &[byte; 32])
}

fn add_category(t: &LendingTest) -> u32 {
    t.gov_client().add_e_mode_category()
}

fn try_add_asset(
    t: &LendingTest,
    asset: &soroban_sdk::Address,
    category_id: u32,
    ltv: u32,
    threshold: u32,
    bonus: u32,
) -> Result<(), soroban_sdk::Error> {
    match t.gov_client().try_add_asset_to_e_mode_category(
        asset,
        &category_id,
        &true,
        &true,
        &ltv,
        &threshold,
        &bonus,
        &0i128,
        &0i128,
    ) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

// threshold (8000) <= ltv (9000) must reject with InvalidLiqThreshold (113).
#[test]
fn test_emode_rejects_threshold_lte_ltv() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let id = add_category(&t);
    let usdc = t.resolve_asset("USDC");
    let result = try_add_asset(&t, &usdc, id, 9000, 8000, 200);
    assert_contract_error(result, errors::INVALID_LIQ_THRESHOLD);
}

#[test]
fn test_emode_accepts_valid_asset_bounds() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let id = add_category(&t);
    assert!(id > 0, "governance forwarder should return a category id");
    let usdc = t.resolve_asset("USDC");
    try_add_asset(&t, &usdc, id, 8000, 8500, 200).expect("valid asset should be accepted");

    let cat = t.ctrl_client().get_e_mode_category(&id);
    let cfg = cat.assets.get(usdc).expect("USDC must be registered");
    assert_eq!(cfg.loan_to_value_bps, 8000);
    assert_eq!(cfg.liquidation_threshold_bps, 8500);
    assert_eq!(cfg.liquidation_bonus_bps, 200);
}

#[test]
fn test_emode_add_asset_via_gov_forwarder() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let id = add_category(&t);
    let usdc = t.resolve_asset("USDC");
    t.gov_client()
        .add_asset_to_e_mode_category(&usdc, &id, &true, &true, &8000, &8500, &200, &0i128, &0i128);

    let cat = t.ctrl_client().get_e_mode_category(&id);
    assert!(
        cat.assets.contains_key(usdc),
        "USDC must be registered in the forwarded category"
    );
}

const UNIT: i128 = 10_000_000;

#[test]
fn test_emode_rejects_spoke_supply_cap_above_hub() {
    let hub_cap = 1_000 * UNIT;
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market_params("USDC", |params| {
            params.supply_cap = hub_cap;
        })
        .build();
    let id = add_category(&t);
    let usdc = t.resolve_asset("USDC");

    let result = match t.gov_client().try_add_asset_to_e_mode_category(
        &usdc,
        &id,
        &true,
        &true,
        &8_000,
        &8_500,
        &200,
        &(2_000 * UNIT),
        &0i128,
    ) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, errors::SPOKE_CAP_EXCEEDS_HUB);
}

#[test]
fn test_update_pool_caps_rejects_hub_below_spoke_via_governance() {
    let hub_cap = 10_000 * UNIT;
    let spoke_cap = 2_000 * UNIT;
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market_params("USDC", |params| {
            params.supply_cap = hub_cap;
        })
        .build();
    let id = add_category(&t);
    let usdc = t.resolve_asset("USDC");
    t.gov_client().add_asset_to_e_mode_category(
        &usdc,
        &id,
        &true,
        &true,
        &8_000,
        &8_500,
        &200,
        &spoke_cap,
        &0i128,
    );

    let result = match t
        .gov_client()
        .try_propose_update_pool_caps(&t.admin(), &usdc, &(500 * UNIT), &0i128, &salt(&t.env, 7))
    {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, errors::SPOKE_CAP_EXCEEDS_HUB);
}
