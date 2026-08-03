
use crate::types::OracleTolerance;
use common::errors::{GenericError, OracleError};
use cvlr::nondet::nondet;
use soroban_sdk::{panic_with_error, Env};

pub(crate) fn midpoint_if_in_band(
    env: &Env,
    anchor: i128,
    primary: i128,
    _tolerance: &OracleTolerance,
) -> i128 {

    let within_band: bool = nondet();
    if !within_band {
        panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
    }
    anchor
        .checked_add(primary)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
        / 2
}
