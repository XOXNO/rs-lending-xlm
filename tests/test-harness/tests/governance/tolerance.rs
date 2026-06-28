//! Oracle tolerance bounds on the `edit_oracle_tolerance` forwarder.

use common::errors::OracleError;
use governance::op::{AdminOperation, EditToleranceArgs};
use soroban_sdk::Address;
use test_harness::{assert_contract_error, usdc_preset, LendingTest};

fn try_tolerance(
    t: &LendingTest,
    asset: &Address,
    tolerance: u32,
) -> Result<(), soroban_sdk::Error> {
    match t.gov_client().try_execute_immediate(
        &t.admin(),
        &AdminOperation::EditOracleTolerance(EditToleranceArgs {
            asset: asset.clone(),
            tolerance,
        }),
    ) {
        Ok(res) => res.map(|_| ()).map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

// MIN_TOLERANCE = 150 BPS.
#[test]
fn test_tolerance_config_rejects_below_min() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let result = try_tolerance(&t, &asset, 10);
    assert_contract_error(result, OracleError::BadLastTolerance as u32);
}

// MAX_TOLERANCE = 5000 BPS.
#[test]
fn test_tolerance_config_rejects_above_max() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let result = try_tolerance(&t, &asset, 6000);
    assert_contract_error(result, OracleError::BadLastTolerance as u32);
}

#[test]
fn test_tolerance_config_accepts_valid_bounds() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let expected = t.gov_iface_client().resolve_oracle_tolerance(&500);
    try_tolerance(&t, &asset, 500).expect("valid tolerance should be accepted");

    let stored = t
        .ctrl_client()
        .get_market_config(&asset)
        .oracle_config
        .tolerance;
    assert_eq!(stored, expected);
}
