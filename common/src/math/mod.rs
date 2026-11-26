//! Fixed-point arithmetic used throughout the protocol. `fp` exposes the
//! `Ray`, `Wad`, and `Bps` newtypes for scale-safe arithmetic. Raw arithmetic
//! functions stay private. Specialized LP invariants and integer AMM reserve algorithms
//! retain wide arithmetic where intermediate values cannot fit these types.

#[cfg(test)]
#[path = "../../tests/math/support.rs"]
pub(crate) mod test_support;

pub mod fp;
mod fp_core;

use soroban_sdk::Env;

/// Rounding for an exact ratio applied to a quantity in its original units.
#[derive(Clone, Copy)]
pub enum RatioRounding {
    Floor,
    Ceil,
}

impl RatioRounding {
    /// Returns `amount * numerator / denominator` with a single rounding, or `None` on a zero
    /// divisor or an `i128` overflow.
    pub fn checked_scale(
        self,
        env: &Env,
        amount: i128,
        numerator: i128,
        denominator: i128,
    ) -> Option<i128> {
        if denominator == 0 {
            return None;
        }
        if let Some(product) = amount.checked_mul(numerator) {
            return match self {
                Self::Floor => fp_core::div_floor_i128(product, denominator),
                Self::Ceil => fp_core::div_ceil_i128(product, denominator),
            };
        }
        let (x, y, d) = fp_core::to_i256_operands(env, amount, numerator, denominator);
        let product = x.mul(&y);
        let nonneg = fp_core::quotient_is_nonnegative(amount, numerator, denominator);
        match self {
            Self::Floor => fp_core::div_floor_i256(env, &product, &d, nonneg),
            Self::Ceil => fp_core::div_ceil_i256(env, &product, &d, nonneg),
        }
        .to_i128()
    }
}

/// Raw subjects and expectations for unit tests, fuzzing and Certora rules only.
#[cfg(any(test, feature = "math-test", feature = "certora"))]
#[doc(hidden)]
pub mod reference {
    pub use super::fp_core::{
        div_by_int_half_up, mul_div_ceil, mul_div_floor, mul_div_floor_saturating, mul_div_half_up,
        rescale_half_up, try_mul_div_half_up,
    };
}
