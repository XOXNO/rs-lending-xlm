//! Type-safe fixed-point arithmetic for the lending protocol.
//!
//! Three precision types -- [`Ray`], [`Wad`], and [`Bps`] -- prevent accidental
//! mixing of precisions at compile time. All arithmetic uses half-up rounding
//! (0.5 rounds away from zero) via the [`fp_core::mul_div_half_up`] primitive.
//!
//! These types are **computation-only**; they never reach on-chain storage.
//! At serialization boundaries, use `from_raw()` / `.raw()` to convert
//! to and from the `i128` fields required by `#[contracttype]` structs.

use core::ops::{Add, AddAssign, Sub, SubAssign};
use soroban_sdk::Env;

use crate::constants::{BPS, RAY, RAY_DECIMALS, WAD, WAD_DECIMALS};
use crate::fp_core;

// ===========================================================================
// Ray -- 27-decimal fixed point (indexes, rates, scaled amounts)
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ray(i128);

impl Ray {
    pub const ONE: Ray = Ray(RAY);
    pub const ZERO: Ray = Ray(0);

    #[inline]
    pub fn from_raw(v: i128) -> Self {
        Ray(v)
    }

    #[inline]
    pub fn raw(self) -> i128 {
        self.0
    }

    /// Multiplies two Ray values: `(a * b + RAY/2) / RAY`.
    pub fn mul(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_half_up(env, self.0, other.0, RAY))
    }

    /// Divides two Ray values: `(a * RAY + b/2) / b`.
    pub fn div(self, env: &Env, other: Ray) -> Ray {
        Ray(fp_core::mul_div_half_up(env, self.0, RAY, other.0))
    }

    /// Divides by a plain integer with half-up rounding (for Taylor series).
    pub fn div_by_int(self, n: i128) -> Ray {
        Ray(fp_core::div_by_int_half_up(self.0, n))
    }

    /// Converts a RAY-precision value to WAD (27 -> 18 decimals).
    /// Use only when the value is truly in RAY precision (e.g., after
    /// `scaled * index` where both are RAY-native).
    pub fn to_wad(self) -> Wad {
        Wad(fp_core::rescale_half_up(self.0, RAY_DECIMALS, WAD_DECIMALS))
    }

    /// Converts a RAY-precision value to asset decimals for token transfers.
    /// The only place precision is lost; use at the transfer boundary.
    pub fn to_asset(self, asset_decimals: u32) -> i128 {
        fp_core::rescale_half_up(self.0, RAY_DECIMALS, asset_decimals)
    }

    /// Upscales a token amount from asset decimals to RAY precision.
    /// Use at the token-entry boundary, before any scaled arithmetic.
    pub fn from_asset(amount: i128, asset_decimals: u32) -> Ray {
        Ray(fp_core::rescale_half_up(
            amount,
            asset_decimals,
            RAY_DECIMALS,
        ))
    }
}

impl Add for Ray {
    type Output = Ray;
    fn add(self, rhs: Ray) -> Ray {
        Ray(self.0 + rhs.0)
    }
}

impl AddAssign for Ray {
    fn add_assign(&mut self, rhs: Ray) {
        self.0 += rhs.0;
    }
}

impl Sub for Ray {
    type Output = Ray;
    fn sub(self, rhs: Ray) -> Ray {
        Ray(self.0 - rhs.0)
    }
}

impl SubAssign for Ray {
    fn sub_assign(&mut self, rhs: Ray) {
        self.0 -= rhs.0;
    }
}

// ===========================================================================
// Wad -- 18-decimal fixed point (USD values, prices, health factor)
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Wad(i128);

impl Wad {
    pub const ONE: Wad = Wad(WAD);
    pub const ZERO: Wad = Wad(0);

    #[inline]
    pub fn from_raw(v: i128) -> Self {
        Wad(v)
    }

    #[inline]
    pub fn raw(self) -> i128 {
        self.0
    }

    /// Multiplies two Wad values: `(a * b + WAD/2) / WAD`.
    pub fn mul(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_half_up(env, self.0, other.0, WAD))
    }

    /// Divides two Wad values: `(a * WAD + b/2) / b`.
    pub fn div(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_half_up(env, self.0, WAD, other.0))
    }

    /// Divides two Wad values, rounding the result DOWN toward zero.
    /// Use when a guaranteed lower bound matters (e.g., the base side of
    /// the liquidation seizure split, so the bonus side is never understated).
    pub fn div_floor(self, env: &Env, other: Wad) -> Wad {
        Wad(fp_core::mul_div_floor(env, self.0, WAD, other.0))
    }

    /// Creates a Wad from a token amount at the given decimal precision.
    /// Upscales losslessly to 18 decimals.
    pub fn from_token(amount: i128, decimals: u32) -> Self {
        Wad(fp_core::rescale_half_up(amount, decimals, WAD_DECIMALS))
    }

    /// Converts a Wad back to a token amount at the given decimal precision.
    /// Downscales with half-up rounding.
    pub fn to_token(self, decimals: u32) -> i128 {
        fp_core::rescale_half_up(self.0, WAD_DECIMALS, decimals)
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
}

impl Add for Wad {
    type Output = Wad;
    fn add(self, rhs: Wad) -> Wad {
        Wad(self.0 + rhs.0)
    }
}

impl AddAssign for Wad {
    fn add_assign(&mut self, rhs: Wad) {
        self.0 += rhs.0;
    }
}

impl Sub for Wad {
    type Output = Wad;
    fn sub(self, rhs: Wad) -> Wad {
        Wad(self.0 - rhs.0)
    }
}

impl SubAssign for Wad {
    fn sub_assign(&mut self, rhs: Wad) {
        self.0 -= rhs.0;
    }
}

// ===========================================================================
// Bps -- basis points (LTV, thresholds, bonuses, fees)
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bps(i128);

impl Bps {
    /// 100% = 10_000 BPS.
    pub const ONE: Bps = Bps(BPS);

    #[inline]
    pub fn from_raw(v: i128) -> Self {
        Bps(v)
    }

    #[inline]
    pub fn raw(self) -> i128 {
        self.0
    }

    /// Converts basis points to a WAD ratio: `8000 BPS -> 0.8 WAD`.
    pub fn to_wad(self, env: &Env) -> Wad {
        Wad(fp_core::mul_div_half_up(env, self.0, WAD, BPS))
    }

    /// Applies a basis-point rate to a raw amount: `amount * bps / 10_000`.
    pub fn apply_to(self, env: &Env, amount: i128) -> i128 {
        fp_core::mul_div_half_up(env, amount, self.0, BPS)
    }

    /// Applies a basis-point rate to a Wad value: `value * (bps / 10_000)`.
    pub fn apply_to_wad(self, env: &Env, value: Wad) -> Wad {
        let ratio = self.to_wad(env);
        value.mul(env, ratio)
    }
}

impl Add for Bps {
    type Output = Bps;
    fn add(self, rhs: Bps) -> Bps {
        Bps(self.0 + rhs.0)
    }
}

impl AddAssign for Bps {
    fn add_assign(&mut self, rhs: Bps) {
        self.0 += rhs.0;
    }
}

impl Sub for Bps {
    type Output = Bps;
    fn sub(self, rhs: Bps) -> Bps {
        Bps(self.0 - rhs.0)
    }
}

impl SubAssign for Bps {
    fn sub_assign(&mut self, rhs: Bps) {
        self.0 -= rhs.0;
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn test_ray_mul_div() {
        let env = Env::default();
        let a = Ray::from_raw(2 * RAY);
        let b = Ray::from_raw(3 * RAY);
        assert_eq!(a.mul(&env, b), Ray::from_raw(6 * RAY));

        let c = Ray::from_raw(6 * RAY);
        let d = Ray::from_raw(3 * RAY);
        assert_eq!(c.div(&env, d), Ray::from_raw(2 * RAY));
    }

    #[test]
    fn test_ray_add_sub() {
        let a = Ray::from_raw(RAY);
        let b = Ray::from_raw(RAY / 2);
        assert_eq!((a + b).raw(), RAY + RAY / 2);
        assert_eq!((a - b).raw(), RAY / 2);
    }

    #[test]
    fn test_ray_div_by_int() {
        let x = Ray::from_raw(7);
        assert_eq!(x.div_by_int(2).raw(), 4); // 3.5 -> 4
    }

    #[test]
    fn test_ray_to_wad() {
        // 1.0 in RAY -> WAD (27 -> 18 decimals).
        let r = Ray::from_raw(RAY); // 1.0 in RAY
        let w = r.to_wad();
        assert_eq!(w.raw(), WAD); // 1.0 in WAD
    }

    #[test]
    fn test_ray_from_asset() {
        // 1.0 XLM (7 decimals) -> RAY.
        let r = Ray::from_asset(10_000_000, 7);
        assert_eq!(r.raw(), RAY); // 1.0 in RAY
    }

    #[test]
    fn test_ray_to_asset() {
        // 1.0 in RAY -> 7-decimal asset.
        let r = Ray::from_raw(RAY);
        assert_eq!(r.to_asset(7), 10_000_000);
    }

    #[test]
    fn test_ray_asset_roundtrip() {
        // from_asset -> to_asset must be identity.
        let original = 12_345_678;
        let ray = Ray::from_asset(original, 7);
        assert_eq!(ray.to_asset(7), original);
    }

    #[test]
    fn test_wad_mul_div() {
        let env = Env::default();
        let price = Wad::from_raw(2 * WAD); // $2.00
        let amount = Wad::from_raw(3 * WAD); // 3.0
        assert_eq!(amount.mul(&env, price), Wad::from_raw(6 * WAD));

        let total = Wad::from_raw(6 * WAD);
        let divisor = Wad::from_raw(3 * WAD);
        assert_eq!(total.div(&env, divisor), Wad::from_raw(2 * WAD));
    }

    #[test]
    fn test_wad_from_token() {
        // 1.0 USDC (6 decimals) -> WAD.
        let w = Wad::from_token(1_000_000, 6);
        assert_eq!(w.raw(), 1_000_000_000_000_000_000);
    }

    #[test]
    fn test_wad_to_token() {
        let w = Wad::from_raw(WAD);
        assert_eq!(w.to_token(6), 1_000_000);
        assert_eq!(w.to_token(7), 10_000_000);
    }

    #[test]
    fn test_wad_min_max() {
        let a = Wad::from_raw(10);
        let b = Wad::from_raw(20);
        assert_eq!(a.min(b), a);
        assert_eq!(a.max(b), b);
    }

    #[test]
    fn test_bps_to_wad() {
        let env = Env::default();
        let ltv = Bps::from_raw(8000); // 80%
        let w = ltv.to_wad(&env);
        // 8000 * WAD / 10000 = 0.8 WAD.
        assert_eq!(w.raw(), 800_000_000_000_000_000);
    }

    #[test]
    fn test_bps_apply_to() {
        let env = Env::default();
        let fee_bps = Bps::from_raw(50); // 0.5%
        let amount = 1_000_000_000;
        let fee = fee_bps.apply_to(&env, amount);
        assert_eq!(fee, 5_000_000); // 0.5% of 1B
    }

    #[test]
    fn test_bps_apply_to_wad() {
        let env = Env::default();
        let threshold = Bps::from_raw(8000); // 80%
        let value = Wad::from_raw(100 * WAD);
        let weighted = threshold.apply_to_wad(&env, value);
        assert_eq!(weighted.raw(), 80 * WAD);
    }

    #[test]
    fn test_ray_one_plus_compound() {
        // Simulate: RAY::ONE + x + term2 = 1.0 + 0.1 + 0.005.
        let x = Ray::from_raw(RAY / 10);
        let term2 = Ray::from_raw(RAY / 200);
        let result = Ray::ONE + x + term2;
        assert_eq!(result.raw(), RAY + RAY / 10 + RAY / 200);
    }

    #[test]
    fn test_ordering() {
        assert!(Ray::ZERO < Ray::ONE);
        assert!(Wad::ZERO < Wad::ONE);
        assert!(Bps::from_raw(5000) < Bps::ONE);
    }
}
