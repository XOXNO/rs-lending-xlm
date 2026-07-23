//! Fixed-point wrappers for RAY, WAD, and BPS protocol math.

use soroban_sdk::{panic_with_error, Env};

use crate::constants::{BPS, RAY, RAY_DECIMALS, WAD, WAD_DECIMALS};
use crate::errors::GenericError;
use crate::math::fp_core;

fn checked_add_raw(env: &Env, a: i128, b: i128) -> i128 {
    a.checked_add(b)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

fn checked_sub_nonneg(env: &Env, a: i128, b: i128) -> i128 {
    if a < 0 || b < 0 || b > a {
        panic_with_error!(env, GenericError::MathOverflow);
    }
    a - b
}

/// D27{U}: raw 1e27 fixed-point value. U is caller context:
/// Token, Share, Index, RatePerYear, or dimensionless.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ray(i128);

impl Ray {
    pub const ONE: Ray = Ray(RAY);
    pub const ZERO: Ray = Ray(0);

    /// Wraps a raw 27-decimal RAY value.
    #[inline]
    pub fn from(v: impl Into<i128>) -> Self {
        Ray(v.into())
    }

    #[inline]
    pub fn raw(self) -> i128 {
        self.0
    }

    /// Multiplies two RAY values with half-up rounding.
    pub fn mul(self, env: &Env, other: Ray) -> Ray {
        // D27{A} * D27{B} / D27{1} -> D27{A*B}.
        Ray(fp_core::mul_div_half_up(env, self.0, other.0, RAY))
    }

    /// Divides two RAY values with half-up rounding.
    pub fn div(self, env: &Env, other: Ray) -> Ray {
        // D27{A} * D27{1} / D27{B} -> D27{A/B}.
        Ray(fp_core::mul_div_half_up(env, self.0, RAY, other.0))
    }

    /// Divides two RAY values with floor rounding for non-negative inputs.
    pub fn div_floor(self, env: &Env, other: Ray) -> Ray {
        // D27{A} * D27{1} / D27{B} -> D27{A/B}.
        Ray(fp_core::mul_div_floor(env, self.0, RAY, other.0))
    }

    /// Divides two RAY values with ceiling rounding for non-negative inputs.
    pub fn div_ceil(self, env: &Env, other: Ray) -> Ray {
        // D27{A} * D27{1} / D27{B} -> D27{A/B}.
        Ray(fp_core::mul_div_ceil(env, self.0, RAY, other.0))
    }

    /// Divides by an integer with half-up rounding.
    pub fn div_by_int(self, n: i128) -> Ray {
        // D27{U} / {n} -> D27{U/n}; e.g. annual rate to per-period rate.
        Ray(fp_core::div_by_int_half_up(self.0, n))
    }

    // D27{U} -> D18{U}; floor/ceil variants choose gate rounding direction.
    /// Converts RAY to WAD with half-up rounding.
    pub fn to_wad(self) -> Wad {
        Wad(fp_core::rescale_half_up(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    pub fn to_wad_floor(self) -> Wad {
        Wad(fp_core::rescale_floor(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    pub fn to_wad_ceil(self) -> Wad {
        Wad(fp_core::rescale_ceil(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    // D27{Token(asset)} -> D{asset_decimals}{Token(asset)}.
    /// Converts RAY to asset units with half-up rounding.
    pub fn to_asset(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_half_up(self.0, RAY_DECIMALS, asset_decimals)
    }

    pub fn to_asset_floor(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_floor(self.0, RAY_DECIMALS, asset_decimals)
    }

    pub fn to_asset_ceil(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_ceil(self.0, RAY_DECIMALS, asset_decimals)
    }

    /// Multiplies two RAY values with floor rounding for non-negative inputs.
    pub fn mul_floor(self, env: &Env, other: Ray) -> Ray {
        // D27{A} * D27{B} / D27{1} -> D27{A*B}.
        Ray(fp_core::mul_div_floor(env, self.0, other.0, RAY))
    }

    /// Multiplies two RAY values with ceiling rounding for non-negative inputs.
    pub fn mul_ceil(self, env: &Env, other: Ray) -> Ray {
        // D27{A} * D27{B} / D27{1} -> D27{A*B}.
        Ray(fp_core::mul_div_ceil(env, self.0, other.0, RAY))
    }

    // D27{1} = numerator / denominator; operands must share one unit.
    /// RAY ratio via half-up.
    pub fn from_fraction(env: &Env, numerator: i128, denominator: i128) -> Ray {
        Ray(fp_core::mul_div_half_up(env, numerator, RAY, denominator))
    }

    // D{asset_decimals}{Token(asset)} -> D27{Token(asset)}.
    /// Converts asset units to RAY with half-up rounding.
    pub fn from_asset(amount: i128, asset_decimals: u32) -> Ray {
        Ray(fp_core::rescale_half_up(
            amount,
            asset_decimals,
            RAY_DECIMALS,
        ))
    }

    /// Subtracts two non-negative RAY values and rejects negative results.
    pub fn checked_sub(self, env: &Env, rhs: Ray) -> Ray {
        Ray(checked_sub_nonneg(env, self.0, rhs.0))
    }

    /// In-place checked subtraction.
    pub fn checked_sub_assign(&mut self, env: &Env, rhs: Ray) {
        *self = self.checked_sub(env, rhs);
    }

    /// Adds two RAY values and maps overflow to `MathOverflow`.
    pub fn checked_add(self, env: &Env, rhs: Ray) -> Ray {
        Ray(checked_add_raw(env, self.0, rhs.0))
    }

    /// In-place checked addition.
    pub fn checked_add_assign(&mut self, env: &Env, rhs: Ray) {
        *self = self.checked_add(env, rhs);
    }
}

/// D18{U}: raw 1e18 fixed-point value. U is caller context:
/// USD, Token, price, or dimensionless.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Wad(i128);

impl Wad {
    pub const ONE: Wad = Wad(WAD);
    pub const ZERO: Wad = Wad(0);

    /// Wraps a raw 18-decimal WAD value.
    #[inline]
    pub fn from(v: impl Into<i128>) -> Self {
        Wad(v.into())
    }

    #[inline]
    pub fn raw(self) -> i128 {
        self.0
    }

    /// Multiplies two WAD values with half-up rounding.
    pub fn mul(self, env: &Env, other: Wad) -> Wad {
        // D18{A} * D18{B} / D18{1} -> D18{A*B}.
        Wad(fp_core::mul_div_half_up(env, self.0, other.0, WAD))
    }

    /// Divides two WAD values with half-up rounding.
    pub fn div(self, env: &Env, other: Wad) -> Wad {
        // D18{A} * D18{1} / D18{B} -> D18{A/B}.
        Wad(fp_core::mul_div_half_up(env, self.0, WAD, other.0))
    }

    /// Divides two WAD values with floor rounding for non-negative inputs.
    pub fn div_floor(self, env: &Env, other: Wad) -> Wad {
        // D18{A} * D18{1} / D18{B} -> D18{A/B}.
        Wad(fp_core::mul_div_floor(env, self.0, WAD, other.0))
    }

    /// Floor divide, saturating at `i128::MAX` (e.g. HF with tiny debt).
    pub fn div_floor_saturating(self, env: &Env, other: Wad) -> Wad {
        // D18{A} * D18{1} / D18{B} -> D18{A/B}.
        Wad(fp_core::mul_div_floor_saturating(env, self.0, WAD, other.0))
    }

    /// Multiplies two WAD values with floor rounding for non-negative inputs.
    pub fn mul_floor(self, env: &Env, other: Wad) -> Wad {
        // D18{A} * D18{B} / D18{1} -> D18{A*B}.
        Wad(fp_core::mul_div_floor(env, self.0, other.0, WAD))
    }

    /// Multiplies two WAD values with ceiling rounding for non-negative inputs.
    pub fn mul_ceil(self, env: &Env, other: Wad) -> Wad {
        // D18{A} * D18{B} / D18{1} -> D18{A*B}.
        Wad(fp_core::mul_div_ceil(env, self.0, other.0, WAD))
    }

    // D{decimals}{U} -> D18{U}; U is caller-supplied asset or price unit.
    /// Converts asset units to WAD with half-up rounding.
    pub fn from_token(amount: i128, decimals: u32) -> Self {
        Wad(fp_core::rescale_half_up(amount, decimals, WAD_DECIMALS))
    }

    /// Converts WAD to asset units with half-up rounding.
    pub fn to_token(self, decimals: u32) -> i128 {
        // D18{U} -> D{decimals}{U}; semantic unit unchanged.
        fp_core::rescale_half_up(self.0, WAD_DECIMALS, decimals)
    }

    pub fn to_token_floor(self, decimals: u32) -> i128 {
        // D18{U} -> D{decimals}{U}; semantic unit unchanged.
        fp_core::rescale_floor(self.0, WAD_DECIMALS, decimals)
    }

    // D18{U} -> D27{U}.
    /// Converts WAD to RAY with half-up rounding.
    pub fn to_ray(self) -> Ray {
        Ray(fp_core::rescale_half_up(self.0, WAD_DECIMALS, RAY_DECIMALS))
    }

    pub fn min(self, other: Wad) -> Wad {
        if self.0 < other.0 {
            self
        } else {
            other
        }
    }

    pub fn max(self, other: Wad) -> Wad {
        if self.0 > other.0 {
            self
        } else {
            other
        }
    }

    /// Adds two WAD values and maps overflow to `MathOverflow`.
    pub fn checked_add(self, env: &Env, rhs: Wad) -> Wad {
        Wad(checked_add_raw(env, self.0, rhs.0))
    }

    /// In-place checked addition.
    pub fn checked_add_assign(&mut self, env: &Env, rhs: Wad) {
        *self = self.checked_add(env, rhs);
    }

    /// Subtracts two non-negative WAD values and rejects negative results.
    pub fn checked_sub(self, env: &Env, rhs: Wad) -> Wad {
        Wad(checked_sub_nonneg(env, self.0, rhs.0))
    }

    /// In-place checked subtraction.
    pub fn checked_sub_assign(&mut self, env: &Env, rhs: Wad) {
        *self = self.checked_sub(env, rhs);
    }
}

/// D4{1}: basis-point ratio, 10_000 == 100%.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bps(i128);

impl Bps {
    pub const ONE: Bps = Bps(BPS);

    /// Wraps a raw basis-point value where `10_000` equals 100%.
    #[inline]
    pub fn from(v: impl Into<i128>) -> Self {
        Bps(v.into())
    }

    #[inline]
    pub fn raw(self) -> i128 {
        self.0
    }

    // D4{1} -> D18{1}.
    pub fn to_wad(self, env: &Env) -> Wad {
        Wad(fp_core::mul_div_half_up(env, self.0, WAD, BPS))
    }

    // Dk{U} * D4{1} / D4{1} -> Dk{U}.
    /// Applies this BPS ratio to an integer amount with half-up rounding.
    pub fn apply_to(self, env: &Env, amount: i128) -> i128 {
        fp_core::mul_div_half_up(env, amount, self.0, BPS)
    }

    /// Flash/strategy fee; positive rate floors to at least 1 raw unit.
    pub fn flash_loan_fee_on(self, env: &Env, amount: i128) -> i128 {
        let fee_amount = self.apply_to(env, amount);
        if self.raw() > 0 && fee_amount == 0 {
            // One raw amount unit in caller context Dk{U}, not a BPS unit.
            1
        } else {
            fee_amount
        }
    }

    pub fn apply_to_wad(self, env: &Env, value: Wad) -> Wad {
        let ratio = self.to_wad(env);
        // D18{U} * D18{1} / D18{1} -> D18{U}.
        value.mul(env, ratio)
    }

    pub fn apply_to_wad_floor(self, env: &Env, value: Wad) -> Wad {
        let ratio = self.to_wad(env);
        // D18{U} * D18{1} / D18{1} -> D18{U}.
        value.mul_floor(env, ratio)
    }

    // D27{U} * D4{1} / D4{1} -> D27{U}.
    pub fn apply_to_ray(self, env: &Env, value: Ray) -> Ray {
        Ray(fp_core::mul_div_half_up(env, value.raw(), self.0, BPS))
    }

    /// Adds two BPS values and maps overflow to `MathOverflow`.
    pub fn checked_add(self, env: &Env, rhs: Bps) -> Bps {
        Bps(checked_add_raw(env, self.0, rhs.0))
    }

    /// Subtracts two non-negative BPS values and rejects negative results.
    pub fn checked_sub(self, env: &Env, rhs: Bps) -> Bps {
        Bps(checked_sub_nonneg(env, self.0, rhs.0))
    }
}

#[cfg(test)]
#[path = "../../tests/math/fp.rs"]
mod tests;
