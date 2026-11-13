//! Stand-in for the part of `soroban-sdk` that the controller liquidation math
//! and `common/src/{math,rates}` use: `Env`, `I256`, `Address`, `Vec`, `Map`,
//! `panic_with_error!` and `assert_with_error!`. Copied from
//! `verification/aeneas/shim` (the rates crate) and extended with the
//! collection types.
//!
//! Charon translates only the *signatures* of items from this crate, so to
//! Aeneas every operation here is an opaque function. Their meaning is given
//! once, in `lean/ControllerAeneas/FunsExternal.lean`: `I256` as exact integer
//! arithmetic with the host's 256-bit overflow traps (as in the rates crate).
//! The Rust bodies below exist only so the crate compiles and are never
//! executed.
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

/// Mirrors `soroban_sdk::assert_with_error!`: aborts unless the condition holds.
#[macro_export]
macro_rules! assert_with_error {
    ($env:expr, $cond:expr, $error:expr) => {{
        if !($cond) {
            $crate::panic_with_error!($env, $error);
        }
    }};
}

// ---------------------------------------------------------------------------
// Collections. Only the methods the included controller files call. Each is a
// host object in Soroban; here every method is opaque and
// `lean/ControllerAeneas/FunsExternal.lean` models `Vec<T>` as a Lean list
// (index 0 = front) and `Map<K, V>` as an association list in key order.
// ---------------------------------------------------------------------------

/// A contract address. Opaque: the verified code only copies and compares it.
#[derive(Clone, Debug)]
pub struct Address {
    _opaque: u64,
}

impl PartialEq for Address {
    fn eq(&self, _other: &Self) -> bool {
        unimplemented!("host function")
    }
}

impl Eq for Address {}

/// `soroban_sdk::Vec<T>`: a host-managed growable array.
#[derive(Clone, Debug)]
pub struct Vec<T> {
    _opaque: core::marker::PhantomData<T>,
}

impl<T> Vec<T> {
    pub fn new(_env: &Env) -> Self {
        unimplemented!("host function")
    }
    pub fn len(&self) -> u32 {
        unimplemented!("host function")
    }
    pub fn is_empty(&self) -> bool {
        unimplemented!("host function")
    }
    pub fn get(&self, _i: u32) -> Option<T> {
        unimplemented!("host function")
    }
    pub fn set(&mut self, _i: u32, _v: T) {
        unimplemented!("host function")
    }
    pub fn remove(&mut self, _i: u32) -> Option<()> {
        unimplemented!("host function")
    }
    pub fn push_back(&mut self, _x: T) {
        unimplemented!("host function")
    }
    pub fn append(&mut self, _other: &Vec<T>) {
        unimplemented!("host function")
    }
    pub fn iter(&self) -> VecIter<T> {
        unimplemented!("host function")
    }
    pub fn contains(&self, _x: &T) -> bool {
        unimplemented!("host function")
    }
}

/// Front-to-back iterator over a `Vec<T>`.
pub struct VecIter<T> {
    _opaque: core::marker::PhantomData<T>,
}

impl<T> Iterator for VecIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        unimplemented!("host function")
    }
}

impl<T> IntoIterator for Vec<T> {
    type Item = T;
    type IntoIter = VecIter<T>;
    fn into_iter(self) -> VecIter<T> {
        unimplemented!("host function")
    }
}

impl<T> IntoIterator for &Vec<T> {
    type Item = T;
    type IntoIter = VecIter<T>;
    fn into_iter(self) -> VecIter<T> {
        unimplemented!("host function")
    }
}

/// `soroban_sdk::Map<K, V>`: a host-managed ordered map.
#[derive(Clone, Debug)]
pub struct Map<K, V> {
    _opaque: core::marker::PhantomData<(K, V)>,
}

impl<K, V> Map<K, V> {
    pub fn new(_env: &Env) -> Self {
        unimplemented!("host function")
    }
    pub fn get(&self, _k: K) -> Option<V> {
        unimplemented!("host function")
    }
    pub fn set(&mut self, _k: K, _v: V) {
        unimplemented!("host function")
    }
    pub fn len(&self) -> u32 {
        unimplemented!("host function")
    }
    pub fn is_empty(&self) -> bool {
        unimplemented!("host function")
    }
    pub fn contains_key(&self, _k: K) -> bool {
        unimplemented!("host function")
    }
    pub fn remove(&mut self, _k: K) -> Option<()> {
        unimplemented!("host function")
    }
    pub fn keys(&self) -> Vec<K> {
        unimplemented!("host function")
    }
    pub fn iter(&self) -> VecIter<(K, V)> {
        unimplemented!("host function")
    }
}
