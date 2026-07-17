//! Certora harness for `controller::oracle::tolerance`.
//! Nondet in-band decision; out-of-band panics; in-band result is midpoint.

use crate::types::OraclePriceFluctuation;
use common::errors::{GenericError, OracleError};
use cvlr::nondet::nondet;
use soroban_sdk::{panic_with_error, Env};

pub(crate) fn calculate_final_price(
    env: &Env,
    anchor: i128,
    primary: i128,
    _tolerance: &OraclePriceFluctuation,
) -> i128 {
    // Band decision is free nondet; out-of-band panics; in-band = midpoint.
    let within_band: bool = nondet();
    if !within_band {
        panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
    }
    anchor
        .checked_add(primary)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
        / 2
}
