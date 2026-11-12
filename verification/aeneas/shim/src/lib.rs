//! Stand-in for the part of `soroban-sdk` that `common/src/math` and
//! `common/src/rates` use: `Env`, `I256` and `panic_with_error!`.
//!
//! Charon translates only the *signatures* of items from this crate, so to
//! Aeneas every `I256` operation is an opaque function. Their meaning is given
//! once, in `lean/RatesAeneas/External.lean`, as exact integer arithmetic with
//! the host's 256-bit overflow traps. That file is the trusted base for the
//! `I256` path; the Rust bodies below exist only so the crate compiles and are
//! never executed.
#![no_std]

/// Stateless environment. Nothing in the verified files reads host state.
#[derive(Clone, Copy, Debug, Default)]
pub struct Env;

/// A contract error, as `#[contracterror]` enums convert into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub code: u32,
}

impl Error {
    pub const fn from_contract_error(code: u32) -> Self {
        Error { code }
    }
}

/// Mirrors `soroban_sdk::panic_with_error!`: aborts the invocation. Aeneas
/// translates the `panic!` to failure, so a proof that a function returns
/// `ok` is a proof that it does not abort. (The error code itself is checked
/// by the Kani harnesses, not here.)
#[macro_export]
macro_rules! panic_with_error {
    ($env:expr, $error:expr) => {{
        let _ = &$env;
        let _ = $error;
        panic!("contract error")
    }};
}

/// 256-bit signed integer backed by host functions. Opaque to Aeneas.
#[derive(Clone, Debug)]
pub struct I256 {
    _opaque: [u64; 4],
}

impl I256 {
    pub fn from_i128(_env: &Env, _i: i128) -> Self {
        unimplemented!("host function")
    }
    pub fn to_i128(&self) -> Option<i128> {
        unimplemented!("host function")
    }
    pub fn add(&self, _other: &I256) -> I256 {
        unimplemented!("host function")
    }
    pub fn sub(&self, _other: &I256) -> I256 {
        unimplemented!("host function")
    }
    pub fn mul(&self, _other: &I256) -> I256 {
        unimplemented!("host function")
    }
    pub fn div(&self, _other: &I256) -> I256 {
        unimplemented!("host function")
    }
    pub fn rem_euclid(&self, _other: &I256) -> I256 {
        unimplemented!("host function")
    }
}

impl PartialEq for I256 {
    fn eq(&self, _other: &Self) -> bool {
        unimplemented!("host function")
    }
}

impl PartialOrd for I256 {
    fn partial_cmp(&self, _other: &Self) -> Option<core::cmp::Ordering> {
        unimplemented!("host function")
    }
}
