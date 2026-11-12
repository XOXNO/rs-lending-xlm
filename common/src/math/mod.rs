//! Fixed-point arithmetic used throughout the protocol. `fp` exposes the
//! `Ray`, `Wad`, and `Bps` newtypes for scale-safe arithmetic. Raw primitives
//! stay private; specialized LP invariants and integer AMM reserve algorithms
//! retain wide arithmetic where intermediate values cannot fit these types.

#[cfg(test)]
#[path = "../../tests/math/support.rs"]
pub(crate) mod test_support;

pub mod fp;
mod fp_core;

/// Raw subjects and expectations for unit tests, fuzzing and Certora rules only.
#[cfg(any(test, feature = "math-test", feature = "certora"))]
#[doc(hidden)]
pub mod reference {
    pub use super::fp_core::{
        div_by_int_half_up, mul_div_ceil, mul_div_floor, mul_div_floor_saturating, mul_div_half_up,
        rescale_half_up, try_mul_div_half_up,
    };
}
