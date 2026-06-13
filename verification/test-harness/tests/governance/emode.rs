//! E-mode risk-bound validation on the governance forwarder.

use test_harness::{assert_contract_error, errors, usdc_preset, LendingTest};

// threshold (8000) <= ltv (9000) must reject with InvalidLiqThreshold (113).
#[test]
fn test_emode_rejects_threshold_lte_ltv() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let flat: Result<(), soroban_sdk::Error> = match t
        .gov_client()
        .try_add_e_mode_category(&9000u32, &8000u32, &200u32)
    {
        Ok(res) => res.map(|_| ()).map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(flat, errors::INVALID_LIQ_THRESHOLD);
}
