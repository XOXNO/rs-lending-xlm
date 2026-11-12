//! Stand-in for the part of `soroban-sdk` that the LP oracle code uses:
//! `Env`, `I256` (through `common/src/math/fp_core.rs`), `U256`,
//! `panic_with_error!` and `assert_with_error!`.
//!
//! Copied from `verification/aeneas/shim` (the rates shim) and extended with
//! `U256` and `assert_with_error!`.
//!
//! Charon translates only the *signatures* of items from this crate, so to
//! Aeneas every `I256`/`U256` operation is an opaque function. Their meaning
//! is given once, in `lean/OracleAeneas/FunsExternal.lean`: `I256` as exact
//! integer arithmetic with the host's signed 256-bit traps, `U256` as exact
//! natural-number arithmetic with the host's unsigned 256-bit traps. That
//! file is the trusted base for the host numbers; the Rust bodies below exist
//! only so the crate compiles and are never executed.
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
/// `ok` is a proof that it does not abort.
#[macro_export]
macro_rules! panic_with_error {
    ($env:expr, $error:expr) => {{
        let _ = &$env;
        let _ = $error;
        panic!("contract error")
    }};
}

/// Mirrors `soroban_sdk::assert_with_error!` (soroban-sdk 28.0.0,
/// `src/lib.rs`): `if !cond { panic_with_error!(env, error) }`.
#[macro_export]
macro_rules! assert_with_error {
    ($env:expr, $cond:expr, $error:expr) => {{
        if !($cond) {
            $crate::panic_with_error!($env, $error);
        }
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

/// 256-bit unsigned integer backed by host functions (soroban-sdk 28.0.0,
/// `src/num.rs`). Opaque to Aeneas.
#[derive(Clone, Debug)]
pub struct U256 {
    _opaque: [u64; 4],
}

impl U256 {
    pub fn from_u32(_env: &Env, _u: u32) -> Self {
        unimplemented!("host function")
    }
    pub fn from_u128(_env: &Env, _u: u128) -> Self {
        unimplemented!("host function")
    }
    pub fn to_u128(&self) -> Option<u128> {
        unimplemented!("host function")
    }
    pub fn add(&self, _other: &U256) -> U256 {
        unimplemented!("host function")
    }
    pub fn sub(&self, _other: &U256) -> U256 {
        unimplemented!("host function")
    }
    pub fn mul(&self, _other: &U256) -> U256 {
        unimplemented!("host function")
    }
    pub fn div(&self, _other: &U256) -> U256 {
        unimplemented!("host function")
    }
    pub fn checked_mul(&self, _other: &U256) -> Option<U256> {
        unimplemented!("host function")
    }
}

impl PartialEq for U256 {
    fn eq(&self, _other: &Self) -> bool {
        unimplemented!("host function")
    }
}

impl PartialOrd for U256 {
    fn partial_cmp(&self, _other: &Self) -> Option<core::cmp::Ordering> {
        unimplemented!("host function")
    }
}
