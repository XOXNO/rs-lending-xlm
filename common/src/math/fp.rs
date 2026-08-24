//! Fixed-point newtypes (`Ray`, `Wad`, `Bps`) built on the raw arithmetic in
//! [`fp_core`]. Each type wraps an `i128` at a fixed decimal scale (27, 18,
//! and 4 decimals respectively) and exposes scale-safe multiplication,
//! division, rescaling between types, and checked addition/subtraction.

use soroban_sdk::{panic_with_error, Env};

use crate::constants::{BPS, RAY, RAY_DECIMALS, WAD, WAD_DECIMALS};
use crate::errors::GenericError;
use crate::math::fp_core;

/// Adds two raw values, panicking with `GenericError::MathOverflow` on overflow.
fn checked_add_raw(env: &Env, a: i128, b: i128) -> i128 {
    a.checked_add(b)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

/// Subtracts `b` from `a`, panicking with `GenericError::MathOverflow` if either
/// operand is negative or the subtraction would go negative.
fn checked_sub_nonneg(env: &Env, a: i128, b: i128) -> i128 {
    if a < 0 || b < 0 || b > a {
        panic_with_error!(env, GenericError::MathOverflow);
    }
    a - b
}

/// A non-negative fixed-point value scaled by `RAY` (27 decimals).
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ray(i128);

impl Ray {
    pub const ONE: Ray = Ray(RAY);
    pub const ZERO: Ray = Ray(0);

    /// Wraps a raw ray-scaled integer.
    #[inline]
    pub fn from(v: impl Into<i128>) -> Self {
        Ray(v.into())
    }

    /// Returns the underlying ray-scaled integer.
    #[inline]
    pub fn raw(self) -> i128 {
        self.0
    }

    /// Multiplies two ray values, rounding the result half up.
    pub fn mul(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_half_up(env, self.0, other.0, RAY))
    }

    /// Divides this value by `other`, rounding the result half up.
    pub fn div(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_half_up(env, self.0, RAY, other.0))
    }

    /// Divides this value by `other`, truncating the result toward zero.
    pub fn div_floor(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_floor(env, self.0, RAY, other.0))
    }

    /// Divides this value by `other`, rounding the result up.
    pub fn div_ceil(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_ceil(env, self.0, RAY, other.0))
    }

    /// Divides this value by the plain integer `n`, rounding half up.
    pub fn div_by_int(self, n: i128) -> Ray {
        Ray(fp_core::div_by_int_half_up(self.0, n))
    }

    /// Multiplies two ray values, truncating the result toward zero.
    pub fn mul_floor(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_floor(env, self.0, other.0, RAY))
    }

    /// Multiplies two ray values, rounding the result up.
    pub fn mul_ceil(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_ceil(env, self.0, other.0, RAY))
    }

    /// Multiplies this value by `numerator / denominator`, rounding the result up.
    pub fn mul_ratio_ceil(self, env: &Env, numerator: i128, denominator: i128) -> Ray {
        Ray(fp_core::mul_div_ceil(env, self.0, numerator, denominator))
    }

    /// Converts to a `Wad`, rounding half up from 27 to 18 decimals.
    pub fn to_wad(self) -> Wad {
        Wad(fp_core::rescale_half_up(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    /// Converts to a `Wad`, truncating toward zero from 27 to 18 decimals.
    pub(crate) fn to_wad_floor(self) -> Wad {
        Wad(fp_core::rescale_floor(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    /// Converts to a `Wad`, rounding up from 27 to 18 decimals.
    pub(crate) fn to_wad_ceil(self) -> Wad {
        Wad(fp_core::rescale_ceil(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    /// Rescales from 27 decimals to `asset_decimals`, rounding half up.
    pub fn to_asset(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_half_up(self.0, RAY_DECIMALS, asset_decimals)
    }

    /// Rescales from 27 decimals to `asset_decimals`, truncating toward zero.
    pub fn to_asset_floor(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_floor(self.0, RAY_DECIMALS, asset_decimals)
    }

    /// Rescales from 27 decimals to `asset_decimals`, rounding up.
    pub fn to_asset_ceil(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_ceil(self.0, RAY_DECIMALS, asset_decimals)
    }

    /// Builds a `Ray` from `numerator / denominator`, rounding the result half up.
    pub fn from_fraction(env: &Env, numerator: i128, denominator: i128) -> Ray {
        Ray(fp_core::mul_div_half_up(env, numerator, RAY, denominator))
    }

    /// Builds a `Ray` from a token amount at `asset_decimals`, rescaling half up to 27 decimals.
    pub fn from_asset(amount: i128, asset_decimals: u32) -> Ray {
        Ray(fp_core::rescale_half_up(
            amount,
            asset_decimals,
            RAY_DECIMALS,
        ))
    }

    /// Subtracts `rhs` from this value. Panics if either operand is negative or the result
    /// would be negative.
    pub fn checked_sub(self, env: &Env, rhs: Ray) -> Ray {
        Ray(checked_sub_nonneg(env, self.0, rhs.0))
    }

    /// Adds `rhs` to this value. Panics on overflow.
    pub fn checked_add(self, env: &Env, rhs: Ray) -> Ray {
        Ray(checked_add_raw(env, self.0, rhs.0))
    }
}

/// A non-negative fixed-point value scaled by `WAD` (18 decimals).
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Wad(i128);

impl Wad {
    pub const ONE: Wad = Wad(WAD);
    pub const ZERO: Wad = Wad(0);

    /// Wraps a raw wad-scaled integer.
    #[inline]
    pub fn from(v: impl Into<i128>) -> Self {
        Wad(v.into())
    }

    /// Returns the underlying wad-scaled integer.
    #[inline]
    pub fn raw(self) -> i128 {
        self.0
    }

    /// Multiplies two wad values, rounding the result half up.
    pub fn mul(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_half_up(env, self.0, other.0, WAD))
    }

    /// Multiplies two wad values, rounding the result half up. Returns `None` if
    /// either operand is negative or the result does not fit in `i128`.
    pub fn try_mul(self, env: &Env, other: Wad) -> Option<Wad> {
        fp_core::try_mul_div_half_up(env, self.0, other.0, WAD).map(Wad)
    }

    /// Divides this value by `other`, rounding the result half up.
    pub fn div(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_half_up(env, self.0, WAD, other.0))
    }

    /// Divides this value by `other`, truncating the result toward zero.
    pub fn div_floor(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_floor(env, self.0, WAD, other.0))
    }

    /// Divides this value by `other`, truncating toward zero and saturating to
    /// `i128::MAX` instead of overflowing.
    pub fn div_floor_saturating(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_floor_saturating(env, self.0, WAD, other.0))
    }

    /// Multiplies two wad values, truncating the result toward zero.
    pub fn mul_floor(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_floor(env, self.0, other.0, WAD))
    }

    /// Multiplies two wad values, rounding the result up.
    pub fn mul_ceil(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_ceil(env, self.0, other.0, WAD))
    }

    /// Builds a `Wad` from a token amount at `decimals`, rescaling half up to 18 decimals.
    pub fn from_token(amount: i128, decimals: u32) -> Self {
        Wad(fp_core::rescale_half_up(amount, decimals, WAD_DECIMALS))
    }

    /// Rescales from 18 decimals to `decimals`, rounding half up.
    pub fn to_token(self, decimals: u32) -> i128 {
        fp_core::rescale_half_up(self.0, WAD_DECIMALS, decimals)
    }

    /// Rescales from 18 decimals to `decimals`, truncating toward zero.
    pub fn to_token_floor(self, decimals: u32) -> i128 {
        fp_core::rescale_floor(self.0, WAD_DECIMALS, decimals)
    }

    /// Converts to a `Ray`, rescaling half up from 18 to 27 decimals.
    pub fn to_ray(self) -> Ray {
        Ray(fp_core::rescale_half_up(self.0, WAD_DECIMALS, RAY_DECIMALS))
    }

    /// Adds `rhs` to this value. Panics on overflow.
    pub fn checked_add(self, env: &Env, rhs: Wad) -> Wad {
        Wad(checked_add_raw(env, self.0, rhs.0))
    }

    /// Subtracts `rhs` from this value. Panics if either operand is negative or the result
    /// would be negative.
    pub fn checked_sub(self, env: &Env, rhs: Wad) -> Wad {
        Wad(checked_sub_nonneg(env, self.0, rhs.0))
    }
}

/// A non-negative value in basis points (1/10,000ths).
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bps(i128);

impl Bps {
    pub const ONE: Bps = Bps(BPS);

    /// Wraps a raw basis-point integer.
    #[inline]
    pub fn from(v: impl Into<i128>) -> Self {
        Bps(v.into())
    }

    /// Returns the underlying basis-point integer.
    #[inline]
    pub fn raw(self) -> i128 {
        self.0
    }

    /// Converts to a `Wad` ratio (basis points divided by 10,000), rounding half up.
    pub fn to_wad(self, env: &Env) -> Wad {
        Wad(fp_core::mul_div_half_up(env, self.0, WAD, BPS))
    }

    /// Applies this basis-point rate to a raw token `amount`, rounding half up.
    pub fn apply_to(self, env: &Env, amount: i128) -> i128 {
        fp_core::mul_div_half_up(env, amount, self.0, BPS)
    }

    /// Applies this basis-point rate to `amount` as a flash-loan fee. Rounds half up, but if
    /// the rate is positive and rounding would produce a zero fee, returns 1 instead.
    pub fn flash_loan_fee_on(self, env: &Env, amount: i128) -> i128 {
        let fee_amount = self.apply_to(env, amount);
        if self.raw() > 0 && fee_amount == 0 {
            1
        } else {
            fee_amount
        }
    }

    /// Applies this basis-point rate to a `Wad` value, rounding half up.
    pub fn apply_to_wad(self, env: &Env, value: Wad) -> Wad {
        let ratio = self.to_wad(env);

        value.mul(env, ratio)
    }

    /// Applies this basis-point rate to a `Wad` value, truncating the result toward zero.
    pub fn apply_to_wad_floor(self, env: &Env, value: Wad) -> Wad {
        let ratio = self.to_wad(env);

        value.mul_floor(env, ratio)
    }

    /// Applies this basis-point rate to a `Ray` value, rounding half up.
    pub fn apply_to_ray(self, env: &Env, value: Ray) -> Ray {
        Ray(fp_core::mul_div_half_up(env, value.raw(), self.0, BPS))
    }

    /// Adds `rhs` to this value. Panics on overflow.
    pub fn checked_add(self, env: &Env, rhs: Bps) -> Bps {
        Bps(checked_add_raw(env, self.0, rhs.0))
    }

    /// Subtracts `rhs` from this value. Panics if either operand is negative or the result
    /// would be negative.
    pub fn checked_sub(self, env: &Env, rhs: Bps) -> Bps {
        Bps(checked_sub_nonneg(env, self.0, rhs.0))
    }
}

#[cfg(test)]
#[path = "../../tests/math/fp.rs"]
mod tests;
