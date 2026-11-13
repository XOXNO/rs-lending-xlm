//! Stand-in for `common::errors::{GenericError, OracleError}`, limited to the
//! variants the included files raise. Codes match `common/src/errors.rs`.
//! The production enums carry `#[contracterror]`, which only adds the
//! conversion into `soroban_sdk::Error` written out below.

use soroban_sdk::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum GenericError {
    MathOverflow = 33,
    DivisionByZero = 55,
}

impl From<GenericError> for Error {
    fn from(e: GenericError) -> Self {
        Error::from_contract_error(e as u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum OracleError {
    PriceFeedStale = 206,
    InvalidPrice = 217,
}

impl From<OracleError> for Error {
    fn from(e: OracleError) -> Self {
        Error::from_contract_error(e as u32)
    }
}
