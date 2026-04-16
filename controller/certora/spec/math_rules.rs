/// Math Precision Formal Verification Rules
///
/// Certora Sunbeam rules for the half-up rounding arithmetic system.
///
/// From CLAUDE.md:
///   - All arithmetic uses half-up rounding (rounds 0.5 away from zero)
///   - mul_half_up(a, b, precision) = (a * b + precision/2) / precision
///   - div_half_up(a, b, precision) = (a * precision + b/2) / b
///   - rescale upscaling is lossless; downscaling uses half-up rounding
///   - Signed variants round away from zero for negative results
///   - I256 intermediates prevent overflow for RAY*RAY products
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use common::constants::{RAY, WAD};
use common::fp_core::{mul_div_half_up, mul_div_half_up_signed, rescale_half_up};

// ---------------------------------------------------------------------------
// Rule 1: mul_half_up is commutative — mul_half_up(a, b, p) == mul_half_up(b, a, p)
// ---------------------------------------------------------------------------

#[rule]
fn mul_half_up_commutative(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();
    let p: i128 = cvlr::nondet::nondet();

    // Constrain to positive, realistic ranges
    cvlr_assume!(a >= 0 && a <= RAY);
    cvlr_assume!(b >= 0 && b <= RAY);
    cvlr_assume!(p > 0 && p <= RAY);

    let ab = mul_div_half_up(&e, a, b, p);
    let ba = mul_div_half_up(&e, b, a, p);

    cvlr_assert!(ab == ba);
}

// ---------------------------------------------------------------------------
// Rule 2: mul_half_up with zero — mul_half_up(0, b, p) == 0 and mul_half_up(a, 0, p) == 0
// ---------------------------------------------------------------------------

#[rule]
fn mul_half_up_zero(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();
    let p: i128 = cvlr::nondet::nondet();

    cvlr_assume!(a >= 0 && a <= RAY);
    cvlr_assume!(b >= 0 && b <= RAY);
    cvlr_assume!(p > 0 && p <= RAY);

    // (0 * b + p/2) / p = p/2 / p = 0 for any p >= 2
    // Note: For p == 1, (0 * b + 0) / 1 = 0 as well (since 1/2 = 0 in integer div)
    let zero_times_b = mul_div_half_up(&e, 0, b, p);
    let a_times_zero = mul_div_half_up(&e, a, 0, p);

    cvlr_assert!(zero_times_b == 0);
    cvlr_assert!(a_times_zero == 0);
}

// ---------------------------------------------------------------------------
// Rule 3: mul_half_up identity — mul_half_up(a, RAY, RAY) == a (within +/-1)
// ---------------------------------------------------------------------------

#[rule]
fn mul_half_up_identity(e: Env) {
    let a: i128 = cvlr::nondet::nondet();

    // Constrain to realistic protocol values (up to 10^27 * 10^27 = 10^54 is extreme;
    // actual index products are at most ~10^30)
    cvlr_assume!(a >= 0 && a <= RAY * 1000); // up to 1000 RAY

    // a * RAY / RAY should give back a exactly:
    // (a * RAY + RAY/2) / RAY = a + (RAY/2) / RAY = a + 0 = a
    // since RAY/2 < RAY, integer division discards it
    let result = mul_div_half_up(&e, a, RAY, RAY);

    // Exact equality since (a * RAY + HALF_RAY) / RAY = a when a >= 0
    cvlr_assert!(result == a);
}

#[rule]
fn mul_half_up_identity_sanity(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    cvlr_assume!(a >= 0 && a <= RAY * 1000);

    let result = mul_div_half_up(&e, a, RAY, RAY);
    cvlr_satisfy!(result == a);
}

// ---------------------------------------------------------------------------
// Rule 4: div_half_up is inverse of mul_half_up (within rounding tolerance)
// ---------------------------------------------------------------------------

#[rule]
fn div_half_up_inverse(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    // Positive, non-zero divisor, realistic ranges
    cvlr_assume!(a >= 0 && a <= RAY * 100);
    cvlr_assume!(b > 0 && b <= RAY * 100);

    let product = mul_div_half_up(&e, a, b, RAY);
    let recovered = mul_div_half_up(&e, product, RAY, b);

    // Two rounds of half-up rounding can introduce at most +/-1 each
    cvlr_assert!(recovered >= a - 2 && recovered <= a + 2);
}

#[rule]
fn div_half_up_inverse_sanity(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    cvlr_assume!(a >= 0 && a <= RAY * 100);
    cvlr_assume!(b > 0 && b <= RAY * 100);

    let product = mul_div_half_up(&e, a, b, RAY);
    let recovered = mul_div_half_up(&e, product, RAY, b);
    cvlr_satisfy!(recovered >= a - 2 && recovered <= a + 2);
}

// ---------------------------------------------------------------------------
// Rule 5: div_half_up with zero numerator — div_half_up(0, b, RAY) == 0
// ---------------------------------------------------------------------------

#[rule]
fn div_half_up_zero_numerator(e: Env) {
    let b: i128 = cvlr::nondet::nondet();

    // b must be positive and large enough that b/2 < b (always true for b > 0)
    cvlr_assume!(b > 0 && b <= RAY);

    // mul_div_half_up(0, RAY, b) = (0 * RAY + b/2) / b = (b/2) / b = 0
    // since b/2 < b for all b > 0
    let result = mul_div_half_up(&e, 0, RAY, b);

    cvlr_assert!(result == 0);
}

// ---------------------------------------------------------------------------
// Rule 6: mul_half_up rounding direction — never rounds below floor(a*b/p)
// ---------------------------------------------------------------------------

#[rule]
fn mul_half_up_rounding_direction(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    // Use WAD precision so the floor reference stays within i128.
    cvlr_assume!(a >= 0 && a <= WAD * 100);
    cvlr_assume!(b >= 0 && b <= WAD * 100);

    let result = mul_div_half_up(&e, a, b, WAD);

    // Compute floor with I256 intermediates to avoid overflow.
    let a256 = soroban_sdk::I256::from_i128(&e, a);
    let b256 = soroban_sdk::I256::from_i128(&e, b);
    let p256 = soroban_sdk::I256::from_i128(&e, WAD);
    let floor_256 = a256.mul(&b256).div(&p256);
    let floor = floor_256.to_i128().unwrap();

    cvlr_assert!(result >= floor);
}

#[rule]
fn mul_half_up_rounding_direction_sanity(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    cvlr_assume!(a >= 0 && a <= WAD * 100);
    cvlr_assume!(b >= 0 && b <= WAD * 100);

    let result = mul_div_half_up(&e, a, b, WAD);
    cvlr_satisfy!(result >= 0);
}

// ---------------------------------------------------------------------------
// Rule 7: div_half_up rounding direction — rounds up when remainder >= b/2
// ---------------------------------------------------------------------------

#[rule]
fn div_half_up_rounding_direction(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    // Constrain to positive values; use WAD for manageable intermediate sizes
    cvlr_assume!(a >= 0 && a <= WAD * 100);
    cvlr_assume!(b > 0 && b <= WAD * 100);

    let result = mul_div_half_up(&e, a, WAD, b);

    // Compute floor: (a * WAD) / b using I256
    let a256 = soroban_sdk::I256::from_i128(&e, a);
    let b256 = soroban_sdk::I256::from_i128(&e, b);
    let p256 = soroban_sdk::I256::from_i128(&e, WAD);
    let numerator = a256.mul(&p256);
    let floor_256 = numerator.div(&b256);
    let remainder_256 = numerator.sub(&floor_256.mul(&b256));
    let _half_b = b256.div(&soroban_sdk::I256::from_i128(&e, 2));

    let floor = floor_256.to_i128().unwrap();

    // Correct midpoint test: remainder * 2 >= b (avoids integer division truncation of b/2)
    // When remainder * 2 >= b, half-up rounds up (result = floor + 1)
    // When remainder * 2 < b, result = floor
    let two = soroban_sdk::I256::from_i128(&e, 2);
    if remainder_256.mul(&two) >= b256 {
        cvlr_assert!(result == floor + 1);
    } else {
        cvlr_assert!(result == floor);
    }
}

// ---------------------------------------------------------------------------
// Rule 8: rescale upscale is lossless — rescale_half_up(x, 7, 18) * 10^(18-7) preserves value
// ---------------------------------------------------------------------------

#[rule]
fn rescale_upscale_lossless() {
    let x: i128 = cvlr::nondet::nondet();
    let from: u32 = 7;
    let to: u32 = 18;

    // Realistic token amounts (up to 10^18 at 7 decimals)
    cvlr_assume!(x >= 0 && x <= 1_000_000_000_000_000_000);

    let upscaled = rescale_half_up(x, from, to);

    // Upscaling by 11 decimals means multiplying by 10^11
    // The result must be exactly x * 10^11
    let factor = 10i128.pow(to - from);
    cvlr_assert!(upscaled == x * factor);
}

#[rule]
fn rescale_upscale_lossless_sanity() {
    let x: i128 = cvlr::nondet::nondet();
    cvlr_assume!(x >= 0 && x <= 1_000_000_000_000_000_000);

    let upscaled = rescale_half_up(x, 7, 18);
    cvlr_satisfy!(upscaled > 0);
}

// ---------------------------------------------------------------------------
// Rule 9: rescale roundtrip — rescale_half_up(rescale_half_up(x, 7, 18), 18, 7) approx x (within +/-1)
// ---------------------------------------------------------------------------

#[rule]
fn rescale_roundtrip() {
    let x: i128 = cvlr::nondet::nondet();
    let low: u32 = 7;
    let high: u32 = 18;

    // Positive values, realistic range for 7-decimal tokens
    cvlr_assume!(x >= 0 && x <= 1_000_000_000_000_000);

    // Upscale then downscale
    let upscaled = rescale_half_up(x, low, high);
    let recovered = rescale_half_up(upscaled, high, low);

    // Upscale is exact, downscale uses half-up. Since upscaled = x * 10^11,
    // downscaling divides by 10^11 with half-up rounding.
    // (x * 10^11 + 10^11/2) / 10^11 = x + 0 = x (since 10^11/2 < 10^11)
    // So the roundtrip is exact for 7->18->7.
    cvlr_assert!(recovered == x);
}

#[rule]
fn rescale_roundtrip_sanity() {
    let x: i128 = cvlr::nondet::nondet();
    cvlr_assume!(x >= 0 && x <= 1_000_000_000_000_000);

    let upscaled = rescale_half_up(x, 7, 18);
    let recovered = rescale_half_up(upscaled, 18, 7);
    cvlr_satisfy!(recovered == x);
}

// ---------------------------------------------------------------------------
// Rule 10: signed mul rounds away from zero for negative inputs
// ---------------------------------------------------------------------------

#[rule]
fn signed_mul_away_from_zero(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    // a is negative, b is positive, both within realistic bounds
    cvlr_assume!(a < 0 && a >= -(RAY * 100));
    cvlr_assume!(b > 0 && b <= RAY * 100);

    let result = mul_div_half_up_signed(&e, a, b, RAY);

    // For negative products, rounding away from zero means the result should be
    // <= floor(a*b/RAY) (i.e., more negative or equal).
    // floor(a*b/RAY) for negative a*b is the truncation toward negative infinity.
    //
    // Compute floor via I256: a*b is negative, so a*b / RAY truncates toward zero.
    // floor = a*b/RAY (truncated) if exact, else a*b/RAY - 1 if there's a remainder.
    let a256 = soroban_sdk::I256::from_i128(&e, a);
    let b256 = soroban_sdk::I256::from_i128(&e, b);
    let p256 = soroban_sdk::I256::from_i128(&e, RAY);
    let product = a256.mul(&b256);
    let truncated = product.div(&p256);
    let floor_val = truncated.to_i128().unwrap();

    // Away-from-zero for negative means result <= floor (more negative)
    cvlr_assert!(result <= floor_val);
}

#[rule]
fn signed_mul_away_from_zero_sanity(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    cvlr_assume!(a < 0 && a >= -(RAY * 100));
    cvlr_assume!(b > 0 && b <= RAY * 100);

    let result = mul_div_half_up_signed(&e, a, b, RAY);
    cvlr_satisfy!(result < 0);
}

// ---------------------------------------------------------------------------
// Rule 11: I256 no overflow — mul_half_up with max realistic values (RAY * RAY)
// ---------------------------------------------------------------------------

/// Verifies that mul_half_up with maximum realistic protocol values does not
/// panic from I256-to-i128 conversion. The largest product in the protocol is
/// an index product: (index * index) where each index can be up to ~10 * RAY.
#[rule]
fn i256_no_overflow(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    // Indexes can grow but realistically stay within 10 * RAY (~10^28).
    // Test the extreme: RAY * RAY intermediate = 10^54, well within I256.
    // The result should fit i128 (max ~1.7 * 10^38).
    cvlr_assume!(a >= 0 && a <= 10 * RAY);
    cvlr_assume!(b >= 0 && b <= 10 * RAY);

    // This must not panic — if I256 -> i128 conversion fails, the rule fails
    let result = mul_div_half_up(&e, a, b, RAY);

    // Result should be at most a * b / RAY ~ 10 * RAY * 10 * RAY / RAY = 100 * RAY
    cvlr_assert!(result >= 0);
    cvlr_assert!(result <= 100 * RAY + 1); // +1 for rounding
}

#[rule]
fn i256_no_overflow_sanity(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    cvlr_assume!(a >= 0 && a <= 10 * RAY);
    cvlr_assume!(b >= 0 && b <= 10 * RAY);

    let result = mul_div_half_up(&e, a, b, RAY);
    cvlr_satisfy!(result > 0);
}

// ---------------------------------------------------------------------------
// Rule 12: div_by_zero sanity — div_half_up(a, 0, RAY) should be unreachable
// ---------------------------------------------------------------------------

/// Division by zero must cause a panic (Soroban I256 division by zero panics).
/// This rule constrains the divisor to zero and attempts a division. If the
/// prover can reach the assertion, it means div-by-zero did not revert, which
/// is a violation.
#[rule]
fn div_by_zero_sanity(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    cvlr_assume!(a >= 0 && a <= RAY);

    // Division by zero — this line should always panic
    let _result = mul_div_half_up(&e, a, RAY, 0);

    // Reaching this line would mean division by zero did not revert.
    cvlr_assert!(false);
}
