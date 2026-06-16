//! E-mode risk-bound validation on the governance forwarder.

use test_harness::{assert_contract_error, errors, eth_preset, usdc_preset, LendingTest};

fn try_add_category(
    t: &LendingTest,
    ltv: u32,
    threshold: u32,
    bonus: u32,
) -> Result<u32, soroban_sdk::Error> {
    match t
        .gov_client()
        .try_add_e_mode_category(&ltv, &threshold, &bonus)
    {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

// threshold (8000) <= ltv (9000) must reject with InvalidLiqThreshold (113).
#[test]
fn test_emode_rejects_threshold_lte_ltv() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let result = try_add_category(&t, 9000, 8000, 200);
    assert_contract_error(result, errors::INVALID_LIQ_THRESHOLD);
}

#[test]
fn test_emode_accepts_valid_category_bounds() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let id = try_add_category(&t, 8000, 8500, 200).expect("valid category should be accepted");
    assert!(id > 0, "governance forwarder should return a category id");

    let cat = t.ctrl_client().get_e_mode_category(&id);
    assert_eq!(cat.loan_to_value_bps, 8000);
    assert_eq!(cat.liquidation_threshold_bps, 8500);
    assert_eq!(cat.liquidation_bonus_bps, 200);
}

#[test]
fn test_emode_add_asset_via_gov_forwarder() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let id = try_add_category(&t, 8000, 8500, 200).expect("setup category");
    let usdc = t.resolve_asset("USDC");
    t.gov_client()
        .add_asset_to_e_mode_category(&usdc, &id, &true, &true);

    let cat = t.ctrl_client().get_e_mode_category(&id);
    assert!(
        cat.assets.contains_key(usdc),
        "USDC must be registered in the forwarded category"
    );
}
