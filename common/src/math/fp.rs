//! Fixed-point wrappers for RAY, WAD, and BPS protocol math.

use core::ops::{Add, AddAssign, Sub, SubAssign};
use soroban_sdk::{panic_with_error, Env};

use super::fp_core;
use crate::constants::{BPS, RAY, RAY_DECIMALS, WAD, WAD_DECIMALS};
use crate::errors::GenericError;

/// Adds two raw fixed-point values, mapping overflow to `MathOverflow`.
fn checked_add_raw(env: &Env, a: i128, b: i128) -> i128 {
    a.checked_add(b)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

/// Subtracts two non-negative raw fixed-point values, rejecting negative
/// results with `MathOverflow`.
fn checked_sub_nonneg(env: &Env, a: i128, b: i128) -> i128 {
    if a < 0 || b < 0 || b > a {
        panic_with_error!(env, GenericError::MathOverflow);
    }
    a - b
}

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
        Ray(fp_core::mul_div_half_up(env, self.0, other.0, RAY))
    }

    /// Divides two RAY values with half-up rounding.
    pub fn div(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_half_up(env, self.0, RAY, other.0))
    }

    /// Divides two RAY values with floor rounding for non-negative inputs.
    pub fn div_floor(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_floor(env, self.0, RAY, other.0))
    }

    /// Divides by an integer with half-up rounding.
    pub fn div_by_int(self, n: i128) -> Ray {
        Ray(fp_core::div_by_int_half_up(self.0, n))
    }

    /// Converts RAY to WAD with half-up rounding.
    pub fn to_wad(self) -> Wad {
        Wad(fp_core::rescale_half_up(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    /// Converts RAY to WAD rounded down for collateral-side gate valuations.
    pub fn to_wad_floor(self) -> Wad {
        Wad(fp_core::rescale_floor(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    /// Converts RAY to WAD rounded up for debt-side gate valuations.
    pub fn to_wad_ceil(self) -> Wad {
        Wad(fp_core::rescale_ceil(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    /// Converts RAY to asset units with half-up rounding.
    pub fn to_asset(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_half_up(self.0, RAY_DECIMALS, asset_decimals)
    }

    /// Converts RAY to asset units rounded down for user credits.
    pub fn to_asset_floor(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_floor(self.0, RAY_DECIMALS, asset_decimals)
    }

    /// Converts RAY to asset units rounded up for user debits.
    pub fn to_asset_ceil(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_ceil(self.0, RAY_DECIMALS, asset_decimals)
    }

    /// Multiplies two RAY values with floor rounding for non-negative inputs.
    pub fn mul_floor(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_floor(env, self.0, other.0, RAY))
    }

    /// Multiplies two RAY values with ceiling rounding for non-negative inputs.
    pub fn mul_ceil(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_ceil(env, self.0, other.0, RAY))
    }

    /// Creates a RAY ratio from two integers with half-up rounding.
    pub fn from_fraction(env: &Env, numerator: i128, denominator: i128) -> Ray {
        Ray(fp_core::mul_div_half_up(env, numerator, RAY, denominator))
    }

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

impl Add for Ray {
    type Output = Ray;
    fn add(self, rhs: Ray) -> Ray {
        Ray(self.0.checked_add(rhs.0).expect("Ray addition overflow"))
    }
}

impl AddAssign for Ray {
    fn add_assign(&mut self, rhs: Ray) {
        self.0 = self.0.checked_add(rhs.0).expect("Ray addition overflow");
    }
}

impl Sub for Ray {
    type Output = Ray;
    fn sub(self, rhs: Ray) -> Ray {
        let result = self.0.checked_sub(rhs.0).expect("Ray subtraction overflow");
        if result < 0 {
            panic!("Ray subtraction underflow (would produce negative)");
        }
        Ray(result)
    }
}

impl SubAssign for Ray {
    fn sub_assign(&mut self, rhs: Ray) {
        *self = *self - rhs;
    }
}

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
        Wad(fp_core::mul_div_half_up(env, self.0, other.0, WAD))
    }

    /// Divides two WAD values with half-up rounding.
    pub fn div(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_half_up(env, self.0, WAD, other.0))
    }

    /// Divides two WAD values with floor rounding for non-negative inputs.
    pub fn div_floor(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_floor(env, self.0, WAD, other.0))
    }

    /// Multiplies two WAD values with floor rounding for non-negative inputs.
    pub fn mul_floor(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_floor(env, self.0, other.0, WAD))
    }

    /// Multiplies two WAD values with ceiling rounding for non-negative inputs.
    pub fn mul_ceil(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_ceil(env, self.0, other.0, WAD))
    }

    /// Converts asset units to WAD with half-up rounding.
    pub fn from_token(amount: i128, decimals: u32) -> Self {
        Wad(fp_core::rescale_half_up(amount, decimals, WAD_DECIMALS))
    }

    /// Converts WAD to asset units with half-up rounding.
    pub fn to_token(self, decimals: u32) -> i128 {
        fp_core::rescale_half_up(self.0, WAD_DECIMALS, decimals)
    }

    /// Converts WAD to asset units rounded down for user credits.
    pub fn to_token_floor(self, decimals: u32) -> i128 {
        fp_core::rescale_floor(self.0, WAD_DECIMALS, decimals)
    }

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

impl Add for Wad {
    type Output = Wad;
    fn add(self, rhs: Wad) -> Wad {
        Wad(self.0.checked_add(rhs.0).expect("Wad addition overflow"))
    }
}

impl AddAssign for Wad {
    fn add_assign(&mut self, rhs: Wad) {
        self.0 = self.0.checked_add(rhs.0).expect("Wad addition overflow");
    }
}

impl Sub for Wad {
    type Output = Wad;
    fn sub(self, rhs: Wad) -> Wad {
        let result = self.0.checked_sub(rhs.0).expect("Wad subtraction overflow");
        if result < 0 {
            panic!("Wad subtraction underflow (would produce negative)");
        }
        Wad(result)
    }
}

impl SubAssign for Wad {
    fn sub_assign(&mut self, rhs: Wad) {
        *self = *self - rhs;
    }
}

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

    /// Converts BPS to a WAD ratio.
    pub fn to_wad(self, env: &Env) -> Wad {
        Wad(fp_core::mul_div_half_up(env, self.0, WAD, BPS))
    }

    /// Applies this BPS ratio to an integer amount with half-up rounding.
    pub fn apply_to(self, env: &Env, amount: i128) -> i128 {
        fp_core::mul_div_half_up(env, amount, self.0, BPS)
    }

    /// Flash-loan and strategy-borrow fee from this BPS rate.
    ///
    /// When the rate is positive but half-up rounding yields zero, returns `1`
    /// so dust amounts still pay a unit fee.
    pub fn flash_loan_fee_on(self, env: &Env, amount: i128) -> i128 {
        let fee_amount = self.apply_to(env, amount);
        if self.raw() > 0 && fee_amount == 0 {
            1
        } else {
            fee_amount
        }
    }

    /// Applies this BPS ratio to a WAD value.
    pub fn apply_to_wad(self, env: &Env, value: Wad) -> Wad {
        let ratio = self.to_wad(env);
        value.mul(env, ratio)
    }

    /// Applies this BPS ratio to a WAD value rounded down for gate valuations.
    pub fn apply_to_wad_floor(self, env: &Env, value: Wad) -> Wad {
        let ratio = self.to_wad(env);
        value.mul_floor(env, ratio)
    }

    /// Applies this BPS ratio to a RAY value.
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

impl Add for Bps {
    type Output = Bps;
    fn add(self, rhs: Bps) -> Bps {
        Bps(self.0.checked_add(rhs.0).expect("Bps addition overflow"))
    }
}

impl AddAssign for Bps {
    fn add_assign(&mut self, rhs: Bps) {
        self.0 = self.0.checked_add(rhs.0).expect("Bps addition overflow");
    }
}

impl Sub for Bps {
    type Output = Bps;
    fn sub(self, rhs: Bps) -> Bps {
        let result = self.0.checked_sub(rhs.0).expect("Bps subtraction overflow");
        if result < 0 {
            panic!("Bps subtraction underflow (would produce negative)");
        }
        Bps(result)
    }
}

impl SubAssign for Bps {
    fn sub_assign(&mut self, rhs: Bps) {
        *self = *self - rhs;
    }
}

#[cfg(test)]
#[path = "../../tests/math/fp.rs"]
mod tests;
