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

#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ray(i128);

impl Ray {
    pub const ONE: Ray = Ray(RAY);
    pub const ZERO: Ray = Ray(0);

    #[inline]
    pub fn from(v: impl Into<i128>) -> Self {
        Ray(v.into())
    }

    #[inline]
    pub fn raw(self) -> i128 {
        self.0
    }

    pub fn mul(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_half_up(env, self.0, other.0, RAY))
    }

    pub fn div(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_half_up(env, self.0, RAY, other.0))
    }

    pub fn div_floor(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_floor(env, self.0, RAY, other.0))
    }

    pub fn div_ceil(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_ceil(env, self.0, RAY, other.0))
    }

    pub fn div_by_int(self, n: i128) -> Ray {
        Ray(fp_core::div_by_int_half_up(self.0, n))
    }

    pub fn mul_floor(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_floor(env, self.0, other.0, RAY))
    }

    pub fn mul_ceil(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_ceil(env, self.0, other.0, RAY))
    }

    pub fn mul_ratio_ceil(self, env: &Env, numerator: i128, denominator: i128) -> Ray {
        Ray(fp_core::mul_div_ceil(env, self.0, numerator, denominator))
    }

    pub fn to_wad(self) -> Wad {
        Wad(fp_core::rescale_half_up(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    pub fn to_wad_floor(self) -> Wad {
        Wad(fp_core::rescale_floor(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    pub fn to_wad_ceil(self) -> Wad {
        Wad(fp_core::rescale_ceil(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    pub fn to_asset(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_half_up(self.0, RAY_DECIMALS, asset_decimals)
    }

    pub fn to_asset_floor(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_floor(self.0, RAY_DECIMALS, asset_decimals)
    }

    pub fn to_asset_ceil(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_ceil(self.0, RAY_DECIMALS, asset_decimals)
    }

    pub fn from_fraction(env: &Env, numerator: i128, denominator: i128) -> Ray {
        Ray(fp_core::mul_div_half_up(env, numerator, RAY, denominator))
    }

    pub fn from_asset(amount: i128, asset_decimals: u32) -> Ray {
        Ray(fp_core::rescale_half_up(
            amount,
            asset_decimals,
            RAY_DECIMALS,
        ))
    }

    pub fn checked_sub(self, env: &Env, rhs: Ray) -> Ray {
        Ray(checked_sub_nonneg(env, self.0, rhs.0))
    }

    pub fn checked_add(self, env: &Env, rhs: Ray) -> Ray {
        Ray(checked_add_raw(env, self.0, rhs.0))
    }
}

#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Wad(i128);

impl Wad {
    pub const ONE: Wad = Wad(WAD);
    pub const ZERO: Wad = Wad(0);

    #[inline]
    pub fn from(v: impl Into<i128>) -> Self {
        Wad(v.into())
    }

    #[inline]
    pub fn raw(self) -> i128 {
        self.0
    }

    pub fn mul(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_half_up(env, self.0, other.0, WAD))
    }

    pub fn try_mul(self, env: &Env, other: Wad) -> Option<Wad> {
        fp_core::try_mul_div_half_up(env, self.0, other.0, WAD).map(Wad)
    }

    pub fn div(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_half_up(env, self.0, WAD, other.0))
    }

    pub fn div_floor(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_floor(env, self.0, WAD, other.0))
    }

    pub fn div_floor_saturating(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_floor_saturating(env, self.0, WAD, other.0))
    }

    pub fn mul_floor(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_floor(env, self.0, other.0, WAD))
    }

    pub fn mul_ceil(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_ceil(env, self.0, other.0, WAD))
    }

    pub fn from_token(amount: i128, decimals: u32) -> Self {
        Wad(fp_core::rescale_half_up(amount, decimals, WAD_DECIMALS))
    }

    pub fn to_token(self, decimals: u32) -> i128 {
        fp_core::rescale_half_up(self.0, WAD_DECIMALS, decimals)
    }

    pub fn to_token_floor(self, decimals: u32) -> i128 {
        fp_core::rescale_floor(self.0, WAD_DECIMALS, decimals)
    }

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

    pub fn checked_add(self, env: &Env, rhs: Wad) -> Wad {
        Wad(checked_add_raw(env, self.0, rhs.0))
    }

    pub fn checked_sub(self, env: &Env, rhs: Wad) -> Wad {
        Wad(checked_sub_nonneg(env, self.0, rhs.0))
    }
}

#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bps(i128);

impl Bps {
    pub const ONE: Bps = Bps(BPS);

    #[inline]
    pub fn from(v: impl Into<i128>) -> Self {
        Bps(v.into())
    }

    #[inline]
    pub fn raw(self) -> i128 {
        self.0
    }

    pub fn to_wad(self, env: &Env) -> Wad {
        Wad(fp_core::mul_div_half_up(env, self.0, WAD, BPS))
    }

    pub fn apply_to(self, env: &Env, amount: i128) -> i128 {
        fp_core::mul_div_half_up(env, amount, self.0, BPS)
    }

    pub fn flash_loan_fee_on(self, env: &Env, amount: i128) -> i128 {
        let fee_amount = self.apply_to(env, amount);
        if self.raw() > 0 && fee_amount == 0 {
            1
        } else {
            fee_amount
        }
    }

    pub fn apply_to_wad(self, env: &Env, value: Wad) -> Wad {
        let ratio = self.to_wad(env);

        value.mul(env, ratio)
    }

    pub fn apply_to_wad_floor(self, env: &Env, value: Wad) -> Wad {
        let ratio = self.to_wad(env);

        value.mul_floor(env, ratio)
    }

    pub fn apply_to_ray(self, env: &Env, value: Ray) -> Ray {
        Ray(fp_core::mul_div_half_up(env, value.raw(), self.0, BPS))
    }

    pub fn checked_add(self, env: &Env, rhs: Bps) -> Bps {
        Bps(checked_add_raw(env, self.0, rhs.0))
    }

    pub fn checked_sub(self, env: &Env, rhs: Bps) -> Bps {
        Bps(checked_sub_nonneg(env, self.0, rhs.0))
    }
}

#[cfg(test)]
#[path = "../../tests/math/fp.rs"]
mod tests;
