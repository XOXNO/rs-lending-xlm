//! Stand-in for `common::errors::GenericError`, limited to the variants the
//! included files raise. Codes match `common/src/errors.rs`.

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
