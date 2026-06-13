//! Oracle tolerance bounds on the `edit_oracle_tolerance` forwarder.

use common::errors::OracleError;
use soroban_sdk::Address;
use test_harness::{assert_contract_error, usdc_preset, LendingTest};

fn try_tolerance(
    t: &LendingTest,
    asset: &Address,
    first: u32,
    last: u32,
) -> Result<(), soroban_sdk::Error> {
    match t
        .gov_client()
        .try_edit_oracle_tolerance(&t.admin(), asset, &first, &last)
    {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

// MIN_FIRST_TOLERANCE = 50 BPS.
#[test]
fn test_tolerance_config_rejects_first_below_min() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let result = try_tolerance(&t, &asset, 10, 500);
    assert_contract_error(result, OracleError::BadFirstTolerance as u32);
}

// MAX_FIRST_TOLERANCE = 5000 BPS.
#[test]
fn test_tolerance_config_rejects_first_above_max() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let result = try_tolerance(&t, &asset, 6000, 7000);
    assert_contract_error(result, OracleError::BadFirstTolerance as u32);
}

// MIN_LAST_TOLERANCE = 150 BPS, first=100 is valid.
#[test]
fn test_tolerance_config_rejects_last_below_min() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let result = try_tolerance(&t, &asset, 100, 100);
    assert_contract_error(result, OracleError::BadLastTolerance as u32);
}

// MAX_LAST_TOLERANCE = 10000 BPS.
#[test]
fn test_tolerance_config_rejects_last_above_max() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let result = try_tolerance(&t, &asset, 200, 11000);
    assert_contract_error(result, OracleError::BadLastTolerance as u32);
}

// last (200) < first (300): the second band must be wider than the first.
#[test]
fn test_tolerance_config_rejects_last_less_than_first() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_market("USDC").asset.clone();
    let result = try_tolerance(&t, &asset, 300, 200);
    assert_contract_error(result, OracleError::BadAnchorTolerances as u32);
}
