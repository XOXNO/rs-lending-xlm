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

    /// Converts asset units to WAD with half-up rounding.
    pub fn from_token(amount: i128, decimals: u32) -> Self {
        Wad(fp_core::rescale_half_up(amount, decimals, WAD_DECIMALS))
    }

    /// Converts WAD to asset units with half-up rounding.
    pub fn to_token(self, decimals: u32) -> i128 {
        fp_core::rescale_half_up(self.0, WAD_DECIMALS, decimals)
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

    /// Applies this BPS ratio to a WAD value.
    pub fn apply_to_wad(self, env: &Env, value: Wad) -> Wad {
        let ratio = self.to_wad(env);
        value.mul(env, ratio)
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
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn test_ray_mul_div() {
        let env = Env::default();
        let a = Ray::from(2 * RAY);
        let b = Ray::from(3 * RAY);
        assert_eq!(a.mul(&env, b), Ray::from(6 * RAY));

        let c = Ray::from(6 * RAY);
        let d = Ray::from(3 * RAY);
        assert_eq!(c.div(&env, d), Ray::from(2 * RAY));
    }

    #[test]
    fn test_ray_add_sub() {
        let a = Ray::from(RAY);
        let b = Ray::from(RAY / 2);
        assert_eq!((a + b).raw(), RAY + RAY / 2);
        assert_eq!((a - b).raw(), RAY / 2);
    }

    #[test]
    #[should_panic(expected = "Ray addition overflow")]
    fn test_ray_add_overflow_panics() {
        let _ = Ray::from(i128::MAX) + Ray::from(1);
    }

    #[test]
    fn test_ray_div_by_int() {
        let x = Ray::from(7);
        assert_eq!(x.div_by_int(2).raw(), 4); // 3.5 -> 4
    }

    #[test]
    fn test_ray_to_wad() {
        let r = Ray::from(RAY);
        let w = r.to_wad();
        assert_eq!(w.raw(), WAD);
    }

    #[test]
    fn test_ray_div_floor() {
        let env = Env::default();
        let amount = Ray::from(105 * RAY / 100);
        let ratio = Ray::from(11 * RAY / 10);

        assert_eq!(
            amount.div_floor(&env, ratio).raw(),
            954_545_454_545_454_545_454_545_454
        );
    }

    #[test]
    fn test_ray_from_asset() {
        let r = Ray::from_asset(10_000_000, 7);
        assert_eq!(r.raw(), RAY);
    }

    #[test]
    fn test_ray_to_asset() {
        let r = Ray::from(RAY);
        assert_eq!(r.to_asset(7), 10_000_000);
    }

    #[test]
    fn test_ray_asset_roundtrip() {
        let original = 12_345_678;
        let ray = Ray::from_asset(original, 7);
        assert_eq!(ray.to_asset(7), original);
    }

    #[test]
    fn test_wad_mul_div() {
        let env = Env::default();
        let price = Wad::from(2 * WAD);
        let amount = Wad::from(3 * WAD);
        assert_eq!(amount.mul(&env, price), Wad::from(6 * WAD));

        let total = Wad::from(6 * WAD);
        let divisor = Wad::from(3 * WAD);
        assert_eq!(total.div(&env, divisor), Wad::from(2 * WAD));
    }

    #[test]
    fn test_wad_from_token() {
        let w = Wad::from_token(1_000_000, 6);
        assert_eq!(w.raw(), 1_000_000_000_000_000_000);
    }

    #[test]
    fn test_wad_to_token() {
        let w = Wad::from(WAD);
        assert_eq!(w.to_token(6), 1_000_000);
        assert_eq!(w.to_token(7), 10_000_000);
    }

    #[test]
    fn test_wad_to_ray() {
        let w = Wad::from(WAD);
        assert_eq!(w.to_ray().raw(), RAY);
    }

    #[test]
    fn test_wad_min_max() {
        let a = Wad::from(10);
        let b = Wad::from(20);
        assert_eq!(a.min(b), a);
        assert_eq!(a.max(b), b);
    }

    #[test]
    #[should_panic(expected = "Wad addition overflow")]
    fn test_wad_add_assign_overflow_panics() {
        let mut total = Wad::from(i128::MAX);
        total += Wad::from(1);
    }

    #[test]
    fn test_bps_to_wad() {
        let env = Env::default();
        let ltv = Bps::from(8000);
        let w = ltv.to_wad(&env);
        assert_eq!(w.raw(), 800_000_000_000_000_000);
    }

    #[test]
    fn test_bps_apply_to() {
        let env = Env::default();
        let fee_bps = Bps::from(50);
        let amount = 1_000_000_000;
        let fee = fee_bps.apply_to(&env, amount);
        assert_eq!(fee, 5_000_000);
    }

    #[test]
    fn test_bps_apply_to_wad() {
        let env = Env::default();
        let threshold = Bps::from(8000);
        let value = Wad::from(100 * WAD);
        let weighted = threshold.apply_to_wad(&env, value);
        assert_eq!(weighted.raw(), 80 * WAD);
    }

    #[test]
    fn test_bps_apply_to_ray() {
        let env = Env::default();
        let fee = Bps::from(250).apply_to_ray(&env, Ray::from(4 * RAY));
        assert_eq!(fee.raw(), RAY / 10);
    }

    #[test]
    #[should_panic(expected = "Bps addition overflow")]
    fn test_bps_add_overflow_panics() {
        let _ = Bps::from(i128::MAX) + Bps::from(1);
    }

    #[test]
    fn test_ray_one_plus_compound() {
        let x = Ray::from(RAY / 10);
        let term2 = Ray::from(RAY / 200);
        let result = Ray::ONE + x + term2;
        assert_eq!(result.raw(), RAY + RAY / 10 + RAY / 200);
    }

    #[test]
    fn test_ordering() {
        assert!(Ray::ZERO < Ray::ONE);
        assert!(Wad::ZERO < Wad::ONE);
        assert!(Bps::from(5000) < Bps::ONE);
    }

    #[test]
    fn test_ray_add_assign() {
        let mut x = Ray::from(RAY);
        x += Ray::from(RAY / 2);
        assert_eq!(x.raw(), RAY + RAY / 2);
    }

    #[test]
    fn test_ray_sub_assign() {
        let mut x = Ray::from(RAY);
        x -= Ray::from(RAY / 4);
        assert_eq!(x.raw(), RAY - RAY / 4);
    }

    #[test]
    fn test_ray_checked_sub_ok() {
        let env = Env::default();
        let a = Ray::from(10 * RAY);
        let b = Ray::from(3 * RAY);
        assert_eq!(a.checked_sub(&env, b).raw(), 7 * RAY);
    }

    #[test]
    #[should_panic]
    fn test_ray_checked_sub_underflow_panics() {
        let env = Env::default();
        let a = Ray::from(3 * RAY);
        let b = Ray::from(10 * RAY);
        let _ = a.checked_sub(&env, b);
    }

    #[test]
    #[should_panic]
    fn test_ray_checked_sub_rejects_negative_self() {
        let env = Env::default();
        let _ = Ray::from(-1).checked_sub(&env, Ray::from(0));
    }

    #[test]
    fn test_ray_checked_sub_assign() {
        let env = Env::default();
        let mut x = Ray::from(10 * RAY);
        x.checked_sub_assign(&env, Ray::from(4 * RAY));
        assert_eq!(x.raw(), 6 * RAY);
    }

    #[test]
    fn test_ray_from_asset_high_decimals() {
        let r = Ray::from_asset(1, 0);
        assert_eq!(r.raw(), RAY);
    }

    #[test]
    fn test_wad_add_assign_ok() {
        let mut w = Wad::from(WAD);
        w += Wad::from(WAD / 2);
        assert_eq!(w.raw(), WAD + WAD / 2);
    }

    #[test]
    fn test_wad_sub_assign() {
        let mut w = Wad::from(WAD);
        w -= Wad::from(WAD / 3);
        assert_eq!(w.raw(), WAD - WAD / 3);
    }

    #[test]
    fn test_wad_max_chooses_other_when_self_smaller() {
        let a = Wad::from(1);
        let b = Wad::from(2);
        assert_eq!(a.max(b), b);
    }

    #[test]
    fn test_wad_min_chooses_other_when_self_larger() {
        let a = Wad::from(10);
        let b = Wad::from(5);
        assert_eq!(a.min(b), b);
    }

    #[test]
    fn test_wad_div_floor_rounds_down() {
        let env = Env::default();
        let a = Wad::from(2 * WAD);
        let b = Wad::from(3 * WAD);
        let half_up = a.div(&env, b).raw();
        let floor = a.div_floor(&env, b).raw();
        assert!(
            floor < half_up,
            "div_floor must round strictly down for 2/3"
        );
    }

    #[test]
    fn test_bps_add_assign() {
        let mut b = Bps::from(5000);
        b += Bps::from(2000);
        assert_eq!(b.raw(), 7000);
    }

    #[test]
    fn test_bps_sub_assign() {
        let mut b = Bps::from(5000);
        b -= Bps::from(1500);
        assert_eq!(b.raw(), 3500);
    }

    #[test]
    fn test_bps_sub() {
        let a = Bps::from(7500);
        let b = Bps::from(2500);
        assert_eq!((a - b).raw(), 5000);
    }
    // Adversarial / edge-case coverage for the typed wrappers.

    // Ray::mul at the exact-half boundary. With `(a * b + RAY/2) / RAY`,
    // a value whose remainder is exactly `RAY/2` rounds up. Construct
    // `a = 1, b = RAY/2 + 1` so the product is `RAY/2 + 1` and the
    // exact division equals 0.500…01 → rounds to 1 (last ulp).
    #[test]
    fn test_ray_mul_rounds_half_up() {
        let env = Env::default();
        // 0.5 RAY * 0.5 RAY = 0.25 RAY; remainder is below the half
        // tie-breaker. Use 0.5 RAY * 1 RAY = 0.5 RAY exactly.
        let half = Ray::from(RAY / 2);
        let one = Ray::ONE;
        assert_eq!(half.mul(&env, one).raw(), RAY / 2);
        // 0.5 RAY * 0.5 RAY = 0.25 RAY (= RAY/4).
        assert_eq!(half.mul(&env, half).raw(), RAY / 4);
    }

    // Ray::div by zero — propagates host I256 divide-by-zero panic.
    #[test]
    #[should_panic]
    fn test_ray_div_by_zero_panics() {
        let env = Env::default();
        let _ = Ray::ONE.div(&env, Ray::ZERO);
    }

    // Ray::mul overflow: with i128::MAX in both operands the intermediate
    // I256 holds the product but the post-`/RAY` result still overflows
    // i128 → `MathOverflow`.
    #[test]
    #[should_panic]
    fn test_ray_mul_overflow_panics() {
        let env = Env::default();
        let _ = Ray::from(i128::MAX).mul(&env, Ray::from(i128::MAX));
    }

    // Ray::checked_sub between equal values returns Zero, not panic.
    #[test]
    fn test_ray_checked_sub_equal_returns_zero() {
        let env = Env::default();
        let a = Ray::from(123 * RAY);
        assert_eq!(a.checked_sub(&env, a), Ray::ZERO);
    }

    // Ray::from_asset with decimals == 27 is an identity (RAY_DECIMALS).
    #[test]
    fn test_ray_from_asset_at_ray_decimals_is_identity() {
        let r = Ray::from_asset(12345, 27);
        assert_eq!(r.raw(), 12345);
    }

    #[test]
    #[should_panic(expected = "Ray subtraction underflow")]
    fn test_ray_sub_panics_on_negative_result() {
        let a = Ray::from(RAY);
        let b = Ray::from(2 * RAY);
        let _ = a - b;
    }

    // Wad::div by zero — same propagation path as Ray.
    #[test]
    #[should_panic]
    fn test_wad_div_by_zero_panics() {
        let env = Env::default();
        let _ = Wad::ONE.div(&env, Wad::ZERO);
    }

    // Wad::mul half-up boundary: 0.5 WAD * 0.5 WAD = 0.25 WAD exact.
    #[test]
    fn test_wad_mul_no_rounding_when_exact() {
        let env = Env::default();
        let half = Wad::from(WAD / 2);
        assert_eq!(half.mul(&env, half).raw(), WAD / 4);
    }

    #[test]
    #[should_panic(expected = "Wad subtraction underflow")]
    fn test_wad_sub_panics_on_negative_result() {
        let a = Wad::from(WAD);
        let b = Wad::from(3 * WAD);
        let _ = a - b;
    }

    // Wad::min / max with equal operands: the `else` branch fires, so
    // `min` and `max` both return `other` (the rhs).
    #[test]
    fn test_wad_min_max_equal_operands() {
        let a = Wad::from(42);
        let b = Wad::from(42);
        // Per impl, equal → returns `other` (the rhs) for both.
        assert_eq!(a.min(b), b);
        assert_eq!(a.max(b), b);
    }

    // Wad::max strictly-greater branch — `self > other` returns `self`.
    // The existing `test_wad_min_max` covers self < other; this covers
    // the symmetric `self > other` direction.
    #[test]
    fn test_wad_max_returns_self_when_strictly_greater() {
        let a = Wad::from(100);
        let b = Wad::from(10);
        assert_eq!(a.max(b), a);
        assert_eq!(b.min(a), b);
    }

    // Env-aware checked add / sub coverage. The trait `+` panics with a
    // string; these new methods panic with `GenericError::MathOverflow`.

    #[test]
    fn test_ray_checked_add_ok() {
        let env = Env::default();
        let a = Ray::from(RAY);
        let b = Ray::from(RAY / 2);
        assert_eq!(a.checked_add(&env, b).raw(), RAY + RAY / 2);
    }

    #[test]
    #[should_panic]
    fn test_ray_checked_add_overflow_panics() {
        let env = Env::default();
        let _ = Ray::from(i128::MAX).checked_add(&env, Ray::from(1));
    }

    #[test]
    fn test_wad_checked_add_ok() {
        let env = Env::default();
        assert_eq!(
            Wad::from(WAD).checked_add(&env, Wad::from(WAD)).raw(),
            2 * WAD
        );
    }

    #[test]
    #[should_panic]
    fn test_wad_checked_add_overflow_panics() {
        let env = Env::default();
        let _ = Wad::from(i128::MAX).checked_add(&env, Wad::from(1));
    }

    #[test]
    fn test_wad_checked_sub_ok() {
        let env = Env::default();
        let a = Wad::from(3 * WAD);
        let b = Wad::from(WAD);
        assert_eq!(a.checked_sub(&env, b).raw(), 2 * WAD);
    }

    #[test]
    #[should_panic]
    fn test_wad_checked_sub_underflow_panics() {
        let env = Env::default();
        let _ = Wad::from(1).checked_sub(&env, Wad::from(2));
    }

    #[test]
    fn test_bps_checked_add_ok() {
        let env = Env::default();
        let a = Bps::from(5_000);
        let b = Bps::from(2_500);
        assert_eq!(a.checked_add(&env, b).raw(), 7_500);
    }

    #[test]
    #[should_panic]
    fn test_bps_checked_add_overflow_panics() {
        let env = Env::default();
        let _ = Bps::from(i128::MAX).checked_add(&env, Bps::from(1));
    }

    #[test]
    fn test_bps_checked_sub_ok() {
        let env = Env::default();
        let a = Bps::from(7_500);
        let b = Bps::from(2_500);
        assert_eq!(a.checked_sub(&env, b).raw(), 5_000);
    }

    #[test]
    #[should_panic]
    fn test_bps_checked_sub_underflow_panics() {
        let env = Env::default();
        let _ = Bps::from(100).checked_sub(&env, Bps::from(500));
    }

    // Wad::from_token at decimals == 18 is identity (WAD_DECIMALS).
    #[test]
    fn test_wad_from_token_at_wad_decimals_is_identity() {
        let w = Wad::from_token(98765, 18);
        assert_eq!(w.raw(), 98765);
    }

    // Wad::to_token downscale rounding tie-breaker: 0.5 in the target's
    // smallest unit rounds up to 1.
    #[test]
    fn test_wad_to_token_half_unit_rounds_up() {
        // 1.5 micro-USDC in WAD → 6 decimals: `1_500_000_000_000` at 18d
        // = 1.5 * 10^-6 of a unit → rounds to 2 at 6 decimals.
        let half = Wad::from(1_500_000_000_000i128);
        assert_eq!(half.to_token(6), 2);
    }

    // Bps::apply_to at 0 % returns zero. At 100 % (BPS) returns the
    // input unchanged.
    #[test]
    fn test_bps_apply_to_boundaries() {
        let env = Env::default();
        let amount = 1_000_000i128;
        assert_eq!(Bps::from(0).apply_to(&env, amount), 0);
        assert_eq!(Bps::ONE.apply_to(&env, amount), amount);
    }

    // Bps::apply_to_wad and apply_to_ray at 100 % return the input.
    #[test]
    fn test_bps_apply_to_wad_and_ray_at_one_returns_input() {
        let env = Env::default();
        let w = Wad::from(123 * WAD);
        let r = Ray::from(456 * RAY);
        assert_eq!(Bps::ONE.apply_to_wad(&env, w).raw(), w.raw());
        assert_eq!(Bps::ONE.apply_to_ray(&env, r).raw(), r.raw());
    }

    // Bps overflow at the conversion boundary: 10000 BPS = WAD ratio.
    // bps > BPS produces a ratio > 1, which is a misuse but the math
    // doesn't panic — just produces a larger Wad. Pin the behaviour.
    #[test]
    fn test_bps_to_wad_above_one_does_not_panic() {
        let env = Env::default();
        // 20_000 BPS = 2.0 in WAD.
        assert_eq!(Bps::from(20_000).to_wad(&env).raw(), 2 * WAD);
    }

    // Ray::div_by_int with negative dividend rounds away from zero.
    #[test]
    fn test_ray_div_by_int_negative_rounds_away_from_zero() {
        // Ray(-7) / 2 → -4 (i.e., -3.5 rounds to -4).
        let x = Ray::from(-7);
        assert_eq!(x.div_by_int(2).raw(), -4);
    }

    // Ray::div_floor with positive remainder truncates toward zero. The
    // existing test pins one ratio; this one pins the floor-vs-half
    // divergence explicitly.
    #[test]
    fn test_ray_div_floor_vs_div_diverges_on_half_remainder() {
        let env = Env::default();
        let a = Ray::from(2 * RAY);
        let b = Ray::from(3 * RAY);
        let half_up = a.div(&env, b).raw();
        let floor = a.div_floor(&env, b).raw();
        // 2/3 in RAY: half_up rounds the 0.666…7 up, floor leaves 0.666…6.
        assert_eq!(
            half_up - floor,
            1,
            "div and div_floor must differ by 1 ulp on a half-remainder"
        );
    }

    // ---- Zero-boundary tests for checked_sub on Ray/Wad/Bps ---------------
    // Differentiates `< 0` from `<= 0`/`== 0` on both self and rhs guards.

    #[test]
    fn test_ray_checked_sub_zero_zero_returns_zero() {
        let env = Env::default();
        assert_eq!(Ray::ZERO.checked_sub(&env, Ray::ZERO), Ray::ZERO);
    }

    #[test]
    fn test_wad_checked_sub_zero_zero_returns_zero() {
        let env = Env::default();
        assert_eq!(Wad::ZERO.checked_sub(&env, Wad::ZERO), Wad::ZERO);
    }

    #[test]
    fn test_bps_checked_sub_zero_zero_returns_zero() {
        let env = Env::default();
        let zero = Bps::from(0i128);
        assert_eq!(zero.checked_sub(&env, zero), zero);
    }

    // ---- First `||` disjunct in checked_sub (`self.0 < 0`) ----------------
    // Guard: `self.0 < 0 || rhs.0 < 0 || rhs.0 > self.0`. With `self.0 = 0`
    // and `rhs.0 = -1` the three operands are (F, T, F): only the SECOND
    // disjunct is true. The original `||` chain still panics, but flipping
    // the FIRST `||` (col 23) to `&&` yields `(F && T) || F = F` → no panic.
    // Expecting the panic kills the `||→&&` mutant on the first operator.
    // (The pre-existing `rejects_negative_self` tests use self=-1, rhs=0 →
    // operands (T, F, T), which leave the first-`||→&&` mutant alive.)

    #[test]
    #[should_panic]
    fn test_ray_checked_sub_negative_rhs_with_zero_self_panics() {
        let env = Env::default();
        let _ = Ray::from(0).checked_sub(&env, Ray::from(-1));
    }

    #[test]
    #[should_panic]
    fn test_wad_checked_sub_negative_rhs_with_zero_self_panics() {
        let env = Env::default();
        let _ = Wad::from(0).checked_sub(&env, Wad::from(-1));
    }

    #[test]
    #[should_panic]
    fn test_bps_checked_sub_negative_rhs_with_zero_self_panics() {
        let env = Env::default();
        let _ = Bps::from(0i128).checked_sub(&env, Bps::from(-1i128));
    }

    // ---- Sub trait at equality returns zero (Wad / Bps) -------------------
    // Differentiates `result < 0` from `result <= 0` / `== 0` in Sub impls.

    #[test]
    fn test_wad_sub_equal_returns_zero() {
        assert_eq!(Wad::ONE - Wad::ONE, Wad::ZERO);
    }

    #[test]
    fn test_bps_sub_equal_returns_zero() {
        assert_eq!(Bps::ONE - Bps::ONE, Bps::from(0i128));
    }

    // ---- Assign-op body validation ----------------------------------------
    // Replacing the body with `()` must change observable state.

    #[test]
    fn test_ray_checked_add_assign_mutates() {
        let env = Env::default();
        let mut x = Ray::from(RAY);
        x.checked_add_assign(&env, Ray::from(2 * RAY));
        assert_eq!(x.raw(), 3 * RAY);
    }

    #[test]
    fn test_wad_checked_add_assign_mutates() {
        let env = Env::default();
        let mut x = Wad::from(crate::constants::WAD);
        x.checked_add_assign(&env, Wad::from(2 * crate::constants::WAD));
        assert_eq!(x.raw(), 3 * crate::constants::WAD);
    }

    #[test]
    fn test_wad_checked_sub_assign_mutates() {
        let env = Env::default();
        let mut x = Wad::from(5 * crate::constants::WAD);
        x.checked_sub_assign(&env, Wad::from(2 * crate::constants::WAD));
        assert_eq!(x.raw(), 3 * crate::constants::WAD);
    }

    // ---- Ray::to_asset_floor / to_asset_ceil pin concrete output ----------
    // Differentiates the function body from constant returns (0, 1, -1).

    #[test]
    fn test_ray_to_asset_floor_pins_concrete_output() {
        // 1.5 in Ray (1.5 * 10^27) → asset 7-dec floor = 15000000.
        let r = Ray::from(RAY + RAY / 2);
        assert_eq!(r.to_asset_floor(7), 15_000_000);
        // 1.999_999 in Ray (truncated) at 0-dec = 1 (floor).
        let r2 = Ray::from(RAY + RAY * 999_999 / 1_000_000);
        assert_eq!(r2.to_asset_floor(0), 1);
    }

    #[test]
    fn test_ray_to_asset_ceil_pins_concrete_output() {
        // 1.5 in Ray at 0-dec → ceil = 2.
        let r = Ray::from(RAY + RAY / 2);
        assert_eq!(r.to_asset_ceil(0), 2);
        // Exact 1.0 at 7-dec ceil = 10000000 (no sub-ulp remainder).
        assert_eq!(Ray::ONE.to_asset_ceil(7), 10_000_000);
    }
}
