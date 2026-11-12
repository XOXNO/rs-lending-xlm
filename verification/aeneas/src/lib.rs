//! The production fixed-point and rate arithmetic in `common/src/math` and
//! `common/src/rates`, compiled verbatim through `#[path]` includes for
//! extraction to Lean with Charon and Aeneas. Nothing in `common/src` is
//! copied or edited. See README.md.
#![no_std]

pub mod constants;
pub mod errors;
pub mod math;
pub mod rates;
pub mod types;
