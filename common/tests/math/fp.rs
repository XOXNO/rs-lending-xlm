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
fn test_wad_div_floor_saturating() {
    let env = Env::default();

    // In-range ratio matches ordinary floor division.
    let total = Wad::from(6 * WAD);
    let divisor = Wad::from(3 * WAD);
    assert_eq!(total.div_floor_saturating(&env, divisor), Wad::from(2 * WAD));

    // A tiny divisor makes the true ratio exceed i128; it saturates instead of
    // overflowing.
    let large = Wad::from(i128::MAX / 2);
    let tiny = Wad::from(1);
    assert_eq!(
        large.div_floor_saturating(&env, tiny),
        Wad::from(i128::MAX)
    );
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
fn test_bps_flash_loan_fee_on_charges_min_unit_when_bps_positive() {
    let env = Env::default();
    let fee_bps = Bps::from(9);
    assert_eq!(fee_bps.apply_to(&env, 1), 0);
    assert_eq!(fee_bps.flash_loan_fee_on(&env, 1), 1);
}

#[test]
fn test_bps_flash_loan_fee_on_allows_zero_when_bps_zero() {
    let env = Env::default();
    assert_eq!(Bps::from(0).flash_loan_fee_on(&env, 1), 0);
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
// Typed wrapper edge cases.

// Ray::mul on exact products (0.5*1, 0.5*0.5); no half-up tie-breaker.
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

// Ray::div by zero propagates the host I256 divide-by-zero panic.
#[test]
#[should_panic]
fn test_ray_div_by_zero_panics() {
    let env = Env::default();
    let _ = Ray::ONE.div(&env, Ray::ZERO);
}

// Ray::mul overflow: i128::MAX in both operands; the I256 intermediate holds
// the product, but the post-`/RAY` result overflows i128 and raises `MathOverflow`.
#[test]
#[should_panic]
fn test_ray_mul_overflow_panics() {
    let env = Env::default();
    let _ = Ray::from(i128::MAX).mul(&env, Ray::from(i128::MAX));
}

// Ray::checked_sub between equal values returns zero, not panic.
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

// Wad::div by zero uses the same propagation path as Ray.
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

// Wad::min / max with equal operands return `other` (the rhs).
#[test]
fn test_wad_min_max_equal_operands() {
    let a = Wad::from(42);
    let b = Wad::from(42);
    // Equal operands return the rhs for both.
    assert_eq!(a.min(b), b);
    assert_eq!(a.max(b), b);
}

// Wad::max strict-greater branch returns `self`; symmetric case to
// `test_wad_min_max`'s self < other.
#[test]
fn test_wad_max_returns_self_when_strictly_greater() {
    let a = Wad::from(100);
    let b = Wad::from(10);
    assert_eq!(a.max(b), a);
    assert_eq!(b.min(a), b);
}

// Env-aware checked add/sub: the trait `+` panics with a string; these methods
// panic with `GenericError::MathOverflow`.

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

// bps > BPS produces a ratio > 1 without panicking.
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

// Ray::div_floor truncates toward zero; this case differs from half-up by
// one ulp.
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

// checked_sub zero-boundary tests for Ray/Wad/Bps.
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

// First `||` disjunct in checked_sub (`self.0 < 0`).
// Guard operands with self=0, rhs=-1 are (F, T, F). If the first `||`
// changes to `&&`, the panic is lost; negative-self tests do not cover it.

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

// Sub trait at equality returns zero (Wad / Bps).
// Differentiates `result < 0` from `result <= 0` / `== 0` in Sub impls.

#[test]
fn test_wad_sub_equal_returns_zero() {
    assert_eq!(Wad::ONE - Wad::ONE, Wad::ZERO);
}

#[test]
fn test_bps_sub_equal_returns_zero() {
    assert_eq!(Bps::ONE - Bps::ONE, Bps::from(0i128));
}

// Assign-op body validation.
// Assignment updates observable state.

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

// Ray::to_asset_floor / to_asset_ceil concrete outputs.
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

// Directional gate-valuation primitives.
// floor < half_up < ceil on any non-exact remainder; equal when exact.

#[test]
fn test_ray_mul_ceil_vs_floor_brackets_half_up() {
    let env = Env::default();
    // 1 ulp * 1 ulp / RAY = sub-ulp remainder: floor 0, half-up 0, ceil 1.
    let ulp = Ray::from(1);
    assert_eq!(ulp.mul_floor(&env, ulp).raw(), 0);
    assert_eq!(ulp.mul(&env, ulp).raw(), 0);
    assert_eq!(ulp.mul_ceil(&env, ulp).raw(), 1);
    // Exact product: all three agree.
    let exact = Ray::from(RAY / 2);
    assert_eq!(
        exact.mul_floor(&env, Ray::ONE).raw(),
        exact.mul_ceil(&env, Ray::ONE).raw()
    );
}

#[test]
fn test_ray_to_wad_floor_and_ceil() {
    // RAY + 1 ulp: floor drops the sub-WAD remainder, ceil keeps it.
    let r = Ray::from(RAY + 1);
    assert_eq!(r.to_wad_floor().raw(), WAD);
    assert_eq!(r.to_wad_ceil().raw(), WAD + 1);
    // Exact value: both agree.
    assert_eq!(Ray::ONE.to_wad_floor().raw(), WAD);
    assert_eq!(Ray::ONE.to_wad_ceil().raw(), WAD);
}

#[test]
fn test_wad_mul_floor_and_ceil_bracket_half_up() {
    let env = Env::default();
    // 1 wei * 1 wei / WAD = sub-wei remainder: floor 0, half-up 0, ceil 1.
    let wei = Wad::from(1);
    assert_eq!(wei.mul_floor(&env, wei).raw(), 0);
    assert_eq!(wei.mul(&env, wei).raw(), 0);
    assert_eq!(wei.mul_ceil(&env, wei).raw(), 1);
    // 2/3-style remainder above the half tie-breaker: floor and ceil
    // bracket half-up by exactly one ulp.
    let a = Wad::from(WAD / 3);
    let floor = a.mul_floor(&env, a).raw();
    let half_up = a.mul(&env, a).raw();
    let ceil = a.mul_ceil(&env, a).raw();
    assert!(floor <= half_up && half_up <= ceil);
    assert_eq!(ceil - floor, 1);
}

#[test]
fn test_wad_to_token_floor_rounds_down() {
    // 1.9999995 units at 6 decimals: half-up rounds to 2_000_000,
    // floor keeps 1_999_999.
    let w = Wad::from(1_999_999_500_000_000_000i128);
    assert_eq!(w.to_token(6), 2_000_000);
    assert_eq!(w.to_token_floor(6), 1_999_999);
    // Exact: identical.
    assert_eq!(Wad::ONE.to_token_floor(6), 1_000_000);
}

#[test]
fn test_bps_apply_to_wad_floor_rounds_down() {
    let env = Env::default();
    // 1 wei at 3333 bps: exact = 0.3333 → floor 0, half-up 0 too;
    // use a value with a .5 boundary: 5 wei at 5000 bps = 2.5.
    let v = Wad::from(5);
    let half = Bps::from(5_000);
    // Half-up rounds to 3; floor rounds to 2.
    assert_eq!(half.apply_to_wad(&env, v).raw(), 3);
    assert_eq!(half.apply_to_wad_floor(&env, v).raw(), 2);

    // Exact input has no remainder.
    let exact = Wad::from(100 * WAD);
    assert_eq!(
        Bps::from(8_000).apply_to_wad_floor(&env, exact).raw(),
        80 * WAD
    );
}

// ===== coverage gap-closure tests =====
// test_fp_uncovered_ops (+7) common/src/math/fp.rs:117-119,295-297,416
#[test]
fn test_ray_from_fraction_builds_ratio() {
    let env = Env::default();
    // 1 / 4 in RAY scale.
    assert_eq!(Ray::from_fraction(&env, 1, 4).raw(), RAY / 4);
}

#[test]
fn test_wad_add_operator() {
    let sum = Wad::from(WAD) + Wad::from(WAD / 2);
    assert_eq!(sum.raw(), WAD + WAD / 2);
}

#[test]
#[should_panic(expected = "Bps subtraction underflow")]
fn test_bps_sub_underflow_panics() {
    let _ = Bps::from(100) - Bps::from(500);
}
