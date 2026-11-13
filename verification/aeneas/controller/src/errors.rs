//! Stand-in for `common::errors`, limited to the variants the included files
//! raise. Codes match `common/src/errors.rs`. (The production enums are
//! `#[contracterror]` types; error codes are not modelled by the Lean proofs,
//! which treat every `panic_with_error!` as an abort.)

use soroban_sdk::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum GenericError {
    AmountMustBePositive = 14,
    InvalidPayments = 16,
    MathOverflow = 33,
    InternalError = 34,
    DivisionByZero = 55,
}

impl From<GenericError> for Error {
    fn from(e: GenericError) -> Self {
        Error::from_contract_error(e as u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CollateralError {
    DebtPositionNotFound = 120,
    CollateralPositionNotFound = 121,
}

impl From<CollateralError> for Error {
    fn from(e: CollateralError) -> Self {
        Error::from_contract_error(e as u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum OracleError {
    OracleNotConfigured = 216,
}

impl From<OracleError> for Error {
    fn from(e: OracleError) -> Self {
        Error::from_contract_error(e as u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SpokeError {
    SpokeMismatch = 310,
}

impl From<SpokeError> for Error {
    fn from(e: SpokeError) -> Self {
        Error::from_contract_error(e as u32)
    }
}
