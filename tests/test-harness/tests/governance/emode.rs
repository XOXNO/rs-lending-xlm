//! E-mode risk-bound validation on the governance forwarder.
//!
//! Risk parameters are per-asset, so bound validation happens when an asset
//! joins a category, not at category creation.

use test_harness::{assert_contract_error, errors, eth_preset, usdc_preset, LendingTest};

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
        .add_asset_to_e_mode_category(&usdc, &id, &true, &true, &8000, &8500, &200);

    let cat = t.ctrl_client().get_e_mode_category(&id);
    assert!(
        cat.assets.contains_key(usdc),
        "USDC must be registered in the forwarded category"
    );
}
