use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::Env;

use crate::constants::{RAY, WAD};
use common::math::fp_core::{mul_div_ceil, mul_div_floor, mul_div_half_up, rescale_half_up};

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

#[rule]
fn mul_half_up_rounding_direction(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=100_000_000_000_000).contains(&a));
    cvlr_assume!((0..=100_000_000_000_000).contains(&b));

    let result = mul_div_half_up(&e, a, b, WAD);

    cvlr_assert!(result * WAD >= a * b - (WAD - 1));
}

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

// ---------------------------------------------------------------------------
// Anti-splitting bounds for the fixed-point primitives — Aave Hub `*Additivity`
// analogue, docs `explanation/aave-v4-audit-comparison.md` §5 V-6.
//
// The roundtrip rules above bound the error of converting a value out and back.
// These bound the error of *splitting* one conversion into two, which is the
// pure-math foundation under the pool's `additivity_*` rules: every share
// conversion in `contracts/pool/src/cache/scale.rs` is one of these three
// primitives, so the pool's one-ray-share slack is exactly the slack proved here.
//
// With `f(a) = (a·b + h) / d` truncated and `h = d/2` (half up), `h = 0` (floor),
// or a +1 correction on a nonzero remainder (ceil):
//
//   floor : sub-additive   — splitting can only lose, never gain
//   ceil  : super-additive — splitting can only cost more, never less
//   half up: two-sided, but the extra half only ever moves the result by one
//
// Derivation for the half-up bound, which is the only non-obvious one. With
// `X = a1·b`, `Y = a2·b` and `h = d/2` truncated, `mul_div_half_up` computes
// `floor((X + h)/d)`, so
//
//   split  = floor((X+h)/d) + floor((Y+h)/d)
//   single = floor((X+Y+h)/d)
//
// Lower: `floor(p)+floor(q) >= floor(p+q) − 1` with `p+q = (X+Y+2h)/d >= (X+Y+h)/d`
//        gives `split >= single − 1`.
// Upper: `floor(p)+floor(q) <= floor(p+q) = floor((X+Y+2h)/d)` and `h/d <= 1/2 < 1`
//        gives `split <= single + 1`.
//
// PROOF STATUS: COMPILE-VERIFIED ONLY — not yet run through the Certora prover.
// ---------------------------------------------------------------------------

/// Half-up `mul_div` is additive to within a single unit in either direction:
/// `|f(a1) + f(a2) − f(a1 + a2)| <= 1`. Two half-roundings can add at most one
/// whole unit against the one half-rounding they replace.
#[rule]
fn split_mul_half_up_bounded(e: Env) {
    let a1: i128 = cvlr::nondet::nondet();
    let a2: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();
    let p: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=RAY).contains(&a1));
    cvlr_assume!((0..=RAY).contains(&a2));
    cvlr_assume!((0..=RAY).contains(&b));
    cvlr_assume!((RAY / 1_000..=RAY * 1_000).contains(&p));

    let split = mul_div_half_up(&e, a1, b, p) + mul_div_half_up(&e, a2, b, p);
    let single = mul_div_half_up(&e, a1 + a2, b, p);

    cvlr_assert!(split <= single + 1);
    cvlr_assert!(split >= single - 1);
}

/// Floor `mul_div` is sub-additive: `f(a1) + f(a2) ∈ [f(a1 + a2) − 1, f(a1 + a2)]`.
/// This is the lemma behind `additivity_supply_split_never_mints_more` (supply
/// shares floor) and `additivity_repay_split_never_burns_more_debt` (repay burn
/// floors): splitting can only round away value, never conjure it.
#[rule]
fn split_mul_div_floor_never_gains(e: Env) {
    let a1: i128 = cvlr::nondet::nondet();
    let a2: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();
    let p: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=RAY).contains(&a1));
    cvlr_assume!((0..=RAY).contains(&a2));
    cvlr_assume!((0..=RAY).contains(&b));
    cvlr_assume!((RAY / 1_000..=RAY * 1_000).contains(&p));

    let split = mul_div_floor(&e, a1, b, p) + mul_div_floor(&e, a2, b, p);
    let single = mul_div_floor(&e, a1 + a2, b, p);

    cvlr_assert!(split <= single);
    cvlr_assert!(split >= single - 1);
}

/// Ceil `mul_div` is super-additive: `f(a1) + f(a2) ∈ [f(a1 + a2), f(a1 + a2) + 1]`.
/// This is the lemma behind `additivity_borrow_split_never_reduces_debt` (borrow
/// shares ceil) and the withdraw/net-settle collateral side: splitting can only
/// charge more, never less.
#[rule]
fn split_mul_div_ceil_never_gains(e: Env) {
    let a1: i128 = cvlr::nondet::nondet();
    let a2: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();
    let p: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=RAY).contains(&a1));
    cvlr_assume!((0..=RAY).contains(&a2));
    cvlr_assume!((0..=RAY).contains(&b));
    cvlr_assume!((RAY / 1_000..=RAY * 1_000).contains(&p));

    let split = mul_div_ceil(&e, a1, b, p) + mul_div_ceil(&e, a2, b, p);
    let single = mul_div_ceil(&e, a1 + a2, b, p);

    cvlr_assert!(split >= single);
    cvlr_assert!(split <= single + 1);
}

/// Upscaling rescale is an exact multiplication, so it is exactly additive:
/// splitting a decimal widening is neither better nor worse. `Ray::from_asset`
/// is this direction for every `asset_decimals <= RAY_DECIMALS`, which is why
/// the pool's additivity slack comes only from the index division.
#[rule]
fn split_rescale_upscale_exact() {
    let a1: i128 = cvlr::nondet::nondet();
    let a2: i128 = cvlr::nondet::nondet();
    let low: u32 = 7;
    let high: u32 = 18;

    cvlr_assume!((0..=WAD).contains(&a1));
    cvlr_assume!((0..=WAD).contains(&a2));

    let split = rescale_half_up(a1, low, high) + rescale_half_up(a2, low, high);
    let single = rescale_half_up(a1 + a2, low, high);

    cvlr_assert!(split == single);
}

/// Downscaling rescale rounds half up, so splitting moves the result by at most
/// one unit of the coarser decimal scale: `|f(a1) + f(a2) − f(a1 + a2)| <= 1`.
#[rule]
fn split_rescale_downscale_bounded() {
    let a1: i128 = cvlr::nondet::nondet();
    let a2: i128 = cvlr::nondet::nondet();
    let high: u32 = 18;
    let low: u32 = 7;

    cvlr_assume!((0..=WAD).contains(&a1));
    cvlr_assume!((0..=WAD).contains(&a2));

    let split = rescale_half_up(a1, high, low) + rescale_half_up(a2, high, low);
    let single = rescale_half_up(a1 + a2, high, low);

    cvlr_assert!(split <= single + 1);
    cvlr_assert!(split >= single - 1);
}
