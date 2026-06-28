//! Certora harness for `controller::oracle::tolerance`.
//!
//! `calculate_final_price` performs fixed-point ratio math that is expensive
//! for the prover. This harness replaces the in-band decision with a sound
//! nondet bool while preserving the panic-on-out-of-band control flow, leaving
//! `oracle/tolerance.rs` untouched. Both feeds are required upstream, so the
//! inputs are concrete and the in-band result is the midpoint.

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
    // Production blends a required primary/anchor pair: the midpoint when the
    // two agree within the single tolerance band, otherwise it reverts. The
    // band decision is a free nondet bool because the branch each call selects
    // is determined by the concrete inputs.
    let within_band: bool = nondet();
    if !within_band {
        panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
    }
    anchor
        .checked_add(primary)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
        / 2
}
