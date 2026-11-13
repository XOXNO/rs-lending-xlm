//! Stand-in for the part of `soroban-sdk` that the pool accounting code and
//! the common math it calls use. Extends `../../shim` (the rates shim, copied,
//! not edited) with the items the pool files name.
//!
//! Charon translates only the *signatures* of items from this crate, so to
//! Aeneas every item here is opaque. Only the items reachable from the
//! extraction roots (see `../Cargo.toml`, `[package.metadata.charon]`) reach
//! Lean; their meaning is given in `lean/PoolAeneas/FunsExternal.lean` and
//! `TypesExternal.lean`. The Rust bodies below exist only so the crate
//! compiles and are never executed. Items that exist only so the verbatim
//! production files type-check (token client, contract invocation, `Vec`,
//! `Symbol`, ...) are never reachable from the extracted functions, and are
//! listed in `docs/pool.md`.
#![no_std]

pub use aeneas_pool_shim_macros::{contracterror, contracttype};

/// Host environment handle. Nothing in the extracted functions reads host
/// state through it; it is only threaded through to the math.
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
/// `ok` is a proof that it does not abort. (The error code itself is not
/// modelled.)
#[macro_export]
macro_rules! panic_with_error {
    ($env:expr, $error:expr) => {{
        let _ = &$env;
        let _ = $error;
        panic!("contract error")
    }};
}

/// Mirrors `soroban_sdk::assert_with_error!`: `if !cond { panic_with_error!(env, error) }`.
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

/// A contract or account address. Opaque: the extracted arithmetic only
/// stores and clones it (market identity, asset id).
#[derive(Debug)]
pub struct Address {
    _opaque: u64,
}

impl Clone for Address {
    fn clone(&self) -> Self {
        unimplemented!("host object")
    }
}

impl PartialEq for Address {
    fn eq(&self, _other: &Self) -> bool {
        unimplemented!("host function")
    }
}
impl Eq for Address {}

impl Address {
    pub fn executable(&self) -> Option<Executable> {
        unimplemented!("host function")
    }
}

/// Executable kind of an address (only for `validation::require_wasm_receiver`).
pub enum Executable {
    Wasm(BytesN<32>),
    StellarAsset,
    Account,
}

#[derive(Clone, Debug)]
pub struct Bytes {
    _opaque: u64,
}

#[derive(Clone, Debug)]
pub struct BytesN<const N: usize> {
    _opaque: u64,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    _opaque: u64,
}

impl Symbol {
    pub fn new(_env: &Env, _s: &str) -> Self {
        unimplemented!("host function")
    }
}

#[derive(Clone, Debug)]
pub struct String {
    _opaque: u64,
}

/// A host value.
#[derive(Clone, Copy, Debug)]
pub struct Val {
    _opaque: u64,
}

pub trait IntoVal<E, T> {
    fn into_val(&self, env: &E) -> T;
}

impl<A, B, C, D, E2, F> IntoVal<Env, Vec<Val>> for (A, B, C, D, E2, F) {
    fn into_val(&self, _env: &Env) -> Vec<Val> {
        unimplemented!("host function")
    }
}

#[derive(Clone, Debug)]
pub struct Vec<T> {
    _marker: core::marker::PhantomData<T>,
}

impl<T> Vec<T> {
    pub fn new(_env: &Env) -> Self {
        unimplemented!("host object")
    }
    pub fn is_empty(&self) -> bool {
        unimplemented!("host function")
    }
}

impl Env {
    pub fn current_contract_address(&self) -> Address {
        unimplemented!("host function")
    }
    pub fn invoke_contract<T>(&self, _contract: &Address, _func: &Symbol, _args: Vec<Val>) -> T {
        unimplemented!("host function")
    }
}

pub mod token {
    //! The SEP-41 token client (only for the outbound transfer and flash-loan
    //! paths, which the proofs do not reach).
    use super::{Address, Env};

    pub struct Client<'a> {
        _env: &'a Env,
    }

    impl<'a> Client<'a> {
        pub fn new(env: &'a Env, _address: &Address) -> Self {
            Client { _env: env }
        }
        pub fn balance(&self, _id: &Address) -> i128 {
            unimplemented!("host function")
        }
        pub fn allowance(&self, _from: &Address, _spender: &Address) -> i128 {
            unimplemented!("host function")
        }
        pub fn transfer(&self, _from: &Address, _to: &Address, _amount: &i128) {
            unimplemented!("host function")
        }
        pub fn transfer_from(&self, _spender: &Address, _from: &Address, _to: &Address, _amount: &i128) {
            unimplemented!("host function")
        }
    }
}

// ---- Added for the verbatim `ops/mod.rs` and `common/src/ttl.rs` ----------

/// Host storage handle (`Env::storage`). Only `ttl::renew_instance` uses it
/// (instance TTL extension); modelled as a no-op in `FunsExternal.lean`.
pub struct Storage {
    _opaque: u64,
}

/// Instance storage (`Storage::instance`).
pub struct Instance {
    _opaque: u64,
}

impl Env {
    pub fn storage(&self) -> Storage {
        unimplemented!("host function")
    }
}

impl Storage {
    pub fn instance(&self) -> Instance {
        unimplemented!("host function")
    }
}

impl Instance {
    pub fn extend_ttl(&self, _threshold: u32, _extend_to: u32) {
        unimplemented!("host function")
    }
}

/// Conversion from a host value (only named in `ops::run_batch`'s bounds).
pub trait TryFromVal<E, V>: Sized {
    fn try_from_val(env: &E, v: &V) -> Result<Self, Error>;
}

impl<T: Clone> Vec<T> {
    pub fn iter(&self) -> core::iter::Empty<T> {
        unimplemented!("host function")
    }
    pub fn contains(&self, _item: &T) -> bool {
        unimplemented!("host function")
    }
    pub fn push_back(&mut self, _item: T) {
        unimplemented!("host function")
    }
}

impl<'a> token::Client<'a> {
    pub fn try_transfer(&self, _from: &Address, _to: &Address, _amount: &i128) -> Result<Result<(), Error>, Result<Error, Error>> {
        unimplemented!("host function")
    }
}
