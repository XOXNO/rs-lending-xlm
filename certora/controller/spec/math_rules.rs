//! Half-up rounding, rescaling, and signed-mul precision rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::Env;

use crate::constants::{RAY, WAD};
use common::math::fp_core::{mul_div_half_up, rescale_half_up};

#[rule]
fn mul_half_up_commutative(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();
    let p: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=RAY).contains(&a));
    cvlr_assume!((0..=RAY).contains(&b));
    cvlr_assume!(p > 0 && p <= RAY);

    let ab = mul_div_half_up(&e, a, b, p);
    let ba = mul_div_half_up(&e, b, a, p);

    cvlr_assert!(ab == ba);
}

#[rule]
fn mul_half_up_zero(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();
    let p: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=RAY).contains(&a));
    cvlr_assume!((0..=RAY).contains(&b));
    cvlr_assume!(p > 0 && p <= RAY);

    let zero_times_b = mul_div_half_up(&e, 0, b, p);
    let a_times_zero = mul_div_half_up(&e, a, 0, p);

    cvlr_assert!(zero_times_b == 0);
    cvlr_assert!(a_times_zero == 0);
}

#[rule]
fn mul_half_up_identity(e: Env) {
    let a: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=RAY * 1000).contains(&a));

    let result = mul_div_half_up(&e, a, RAY, RAY);

    cvlr_assert!(result == a);
}

#[rule]
fn div_half_up_roundtrip_error_bounded(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=RAY * 100).contains(&a));
    cvlr_assume!((RAY / 1_000..=RAY * 100).contains(&b));

    let product = mul_div_half_up(&e, a, b, RAY);
    let recovered = mul_div_half_up(&e, product, RAY, b);

    // The first half-up error is magnified by at most RAY / b <= 1000;
    // the second half-up contributes at most one additional raw unit.
    cvlr_assert!(recovered >= a.saturating_sub(501));
    cvlr_assert!(recovered <= a + 501);
}

#[rule]
fn div_half_up_zero_numerator(e: Env) {
    let b: i128 = cvlr::nondet::nondet();

    cvlr_assume!(b > 0 && b <= RAY);

    let result = mul_div_half_up(&e, 0, RAY, b);

    cvlr_assert!(result == 0);
}

/// Half-up multiply never rounds below the mathematical floor.
#[rule]
fn mul_half_up_rounding_direction(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=100_000_000_000_000).contains(&a));
    cvlr_assume!((0..=100_000_000_000_000).contains(&b));

    let result = mul_div_half_up(&e, a, b, WAD);

    cvlr_assert!(result * WAD >= a * b - (WAD - 1));
}

/// Half-up divide stays within one unit of the integer floor.
#[rule]
fn div_half_up_rounding_direction(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=100_000_000_000_000).contains(&a));
    cvlr_assume!(b > 0 && b <= 100_000_000_000_000);

    let result = mul_div_half_up(&e, a, WAD, b);

    cvlr_assert!(result * b >= a * WAD - (b - 1));
    cvlr_assert!(result * b <= a * WAD + b);
}

#[rule]
fn rescale_upscale_lossless() {
    let x: i128 = cvlr::nondet::nondet();
    let from: u32 = 7;
    let to: u32 = 18;

    cvlr_assume!((0..=WAD).contains(&x));

    let upscaled = rescale_half_up(x, from, to);

    let factor = 10i128.pow(to - from);
    cvlr_assert!(upscaled == x * factor);
}

#[rule]
fn rescale_roundtrip() {
    let x: i128 = cvlr::nondet::nondet();
    let low: u32 = 7;
    let high: u32 = 18;

    cvlr_assume!((0..=1_000_000_000_000_000).contains(&x));

    let upscaled = rescale_half_up(x, low, high);
    let recovered = rescale_half_up(upscaled, high, low);

    cvlr_assert!(recovered == x);
}

/// Realistic `RAY`-scale products do not overflow the conversion path.
#[rule]
fn i256_no_overflow(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=10 * RAY).contains(&a));
    cvlr_assume!((0..=10 * RAY).contains(&b));

    let result = mul_div_half_up(&e, a, b, RAY);

    cvlr_assert!(result >= 0);
    cvlr_assert!(result <= 100 * RAY + 1);
}

#[rule]
fn div_by_zero_sanity(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    cvlr_assume!((0..=RAY).contains(&a));

    let _result = mul_div_half_up(&e, a, RAY, 0);

    cvlr_assert!(false);
}
