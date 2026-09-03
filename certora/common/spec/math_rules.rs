use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use crate::constants::{BPS, RAY, WAD};
use crate::math::fp::{Bps, Ray, Wad};
use crate::math::fp_core::{mul_div_ceil, mul_div_floor, mul_div_half_up, rescale_half_up};

/// Largest `x` for which `fp_core::try_mul_div_half_up(x, RAY, RAY)` stays on
/// the native `i128` fast path.
///
/// `mul_div_half_up(x, y, d)` takes the native branch exactly when
/// `x * y + d / 2` fits `i128` and widens to `I256` otherwise
/// (`common/src/math/fp_core.rs`). With `y == d == RAY` the branch condition is
/// `x * RAY + RAY / 2 <= i128::MAX`, which is `x <= (i128::MAX - RAY / 2) / RAY`
/// exactly: below the bound `x * RAY <= i128::MAX - RAY / 2`, and one above it
/// `x * RAY` already exceeds `i128::MAX - RAY / 2`.
const RAY_SQUARE_NATIVE_MAX: i128 = (i128::MAX - RAY / 2) / RAY;

/// `Bps::to_wad` is `mul_div_half_up(bps, WAD, BPS)`, and `BPS` divides `WAD`
/// exactly, so the ratio is `bps * (WAD / BPS)` with no rounding. The product
/// `bps * WAD` is at most `1e4 * 1e18 = 1e22`, far inside `i128`, so that step
/// never leaves the native path and needs no lemma split of its own.
fn bps_ratio_wad(bps: i128) -> i128 {
    bps * (WAD / BPS)
}

/// Largest `value` for which `mul_div_floor(value, ratio, WAD)` stays native.
///
/// `mul_div_floor` has no rounding bias, so its branch condition is just
/// `value * ratio` fitting `i128`. `ratio.max(1)` keeps the bound total: at
/// `ratio == 0` the product is zero for every `value`, and `i128::MAX / 1` then
/// admits the whole domain, which is correct because that case is always native.
fn wad_floor_native_max(ratio: i128) -> i128 {
    i128::MAX / ratio.max(1)
}

/// Native half of `ray_mul_identity`: `amount * RAY + RAY / 2` fits `i128`, so
/// both multiplications run entirely in `i128`.
///
/// Lemma split of the former `ray_mul_identity`. Together with
/// `ray_mul_identity_widened` the two domains partition `0..=10 * RAY` exactly,
/// and each asserts the original identity on its half.
#[rule]
fn ray_mul_identity_native(e: Env, amount: i128) {
    cvlr_assume!((0..=10 * RAY).contains(&amount));
    cvlr_assume!(amount <= RAY_SQUARE_NATIVE_MAX);

    let value = Ray::from(amount);
    cvlr_assert!(value.mul(&e, Ray::ONE).raw() == amount);
    cvlr_assert!(Ray::ONE.mul(&e, value).raw() == amount);
}

/// Widened half of `ray_mul_identity`: the biased product overflows `i128`, so
/// both multiplications go through the exact `I256` host path.
#[rule]
fn ray_mul_identity_widened(e: Env, amount: i128) {
    cvlr_assume!((0..=10 * RAY).contains(&amount));
    cvlr_assume!(amount > RAY_SQUARE_NATIVE_MAX);

    let value = Ray::from(amount);
    cvlr_assert!(value.mul(&e, Ray::ONE).raw() == amount);
    cvlr_assert!(Ray::ONE.mul(&e, value).raw() == amount);
}

/// Native half of `ray_div_floor_never_exceeds_half_up`.
///
/// `Ray::div` is `mul_div_half_up(amount, RAY, divisor)` and `Ray::div_floor`
/// is `mul_div_floor(amount, RAY, divisor)`. The half-up branch condition is
/// the stricter of the two (it carries the `divisor / 2` bias), so assuming it
/// puts *both* calls on the native path.
#[rule]
fn ray_div_floor_never_exceeds_half_up_native(e: Env, amount: i128, divisor: i128) {
    cvlr_assume!((0..=10 * RAY).contains(&amount));
    cvlr_assume!((1..=10 * RAY).contains(&divisor));
    cvlr_assume!(amount <= (i128::MAX - divisor / 2) / RAY);

    let half_up = Ray::from(amount).div(&e, Ray::from(divisor));
    let floor = Ray::from(amount).div_floor(&e, Ray::from(divisor));
    cvlr_assert!(floor.raw() <= half_up.raw());
}

/// Widened half of `ray_div_floor_never_exceeds_half_up`.
///
/// The negation of the native lemma's bound, so the union of the two domains is
/// the original `0..=10 * RAY` by `1..=10 * RAY`. The half-up call is widened
/// here by construction; the floor call may still take its native branch for at
/// most `divisor / 2 / RAY <= 5` values of `amount`, which is why the split is
/// stated on the half-up condition rather than on two independent ones.
#[rule]
fn ray_div_floor_never_exceeds_half_up_widened(e: Env, amount: i128, divisor: i128) {
    cvlr_assume!((0..=10 * RAY).contains(&amount));
    cvlr_assume!((1..=10 * RAY).contains(&divisor));
    cvlr_assume!(amount > (i128::MAX - divisor / 2) / RAY);

    let half_up = Ray::from(amount).div(&e, Ray::from(divisor));
    let floor = Ray::from(amount).div_floor(&e, Ray::from(divisor));
    cvlr_assert!(floor.raw() <= half_up.raw());
}

#[rule]
fn ray_asset_roundtrip_preserves_7_decimal_amount(e: Env, amount: i128) {
    cvlr_assume!((0..=1_000_000_000_000_000i128).contains(&amount));

    let ray = Ray::from_asset(&e, amount, 7);
    cvlr_assert!(ray.to_asset(&e, 7) == amount);
}

#[rule]
fn wad_token_roundtrip_preserves_7_decimal_amount(e: Env, amount: i128) {
    cvlr_assume!((0..=1_000_000_000_000_000i128).contains(&amount));

    let wad = Wad::from_token(&e, amount, 7);
    cvlr_assert!(wad.to_token(&e, 7) == amount);
}

#[rule]
fn wad_to_ray_preserves_one(e: Env) {
    cvlr_assert!(Wad::ONE.to_ray(&e).raw() == RAY);
}

/// No lemma split: `apply_to_ray` is `mul_div_half_up(value, bps, BPS)` and the
/// bounds cap the product at `100 RAY * BPS = 1e33`, more than five orders of
/// magnitude below `i128::MAX`. The widened branch is unreachable on this
/// domain, so the rule already sees exactly one arithmetic path.
#[rule]
fn bps_apply_to_ray_is_bounded(e: Env, value: i128, bps: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&value));
    cvlr_assume!((0..=BPS).contains(&bps));

    let out = Bps::from(bps).apply_to_ray(&e, Ray::from(value));
    cvlr_assert!(out.raw() >= 0);
    cvlr_assert!(out.raw() <= value);
}

/// No lemma split: `Bps::ONE.apply_to_wad` is `mul_div_half_up(value, WAD, WAD)`
/// and `value <= 100 WAD = 1e20` caps the biased product at
/// `1e20 * 1e18 + 5e17 < 1.7014e38 = i128::MAX`. The widened branch is
/// unreachable on this domain.
#[rule]
fn bps_one_is_identity_on_wad(e: Env, value: i128) {
    cvlr_assume!((0..=100 * WAD).contains(&value));

    let out = Bps::ONE.apply_to_wad(&e, Wad::from(value));
    cvlr_assert!(out.raw() == value);
}

#[rule]
fn common_math_reachability(e: Env, amount: i128) {
    cvlr_assume!(amount > 0 && amount <= RAY);
    let out = Ray::from(amount).mul(&e, Ray::ONE);
    cvlr_satisfy!(out.raw() > 0);
}

/// Native half of `bps_apply_to_wad_floor_le_value`: `value * ratio` fits
/// `i128`, so `Wad::mul_floor` runs entirely in `i128`.
#[rule]
fn bps_apply_to_wad_floor_le_value_native(e: Env, value: i128, bps: i128) {
    cvlr_assume!((0..=1_000_000 * WAD).contains(&value));
    cvlr_assume!((0..=BPS).contains(&bps));
    cvlr_assume!(value <= wad_floor_native_max(bps_ratio_wad(bps)));

    let out = Bps::from(bps).apply_to_wad_floor(&e, Wad::from(value));
    cvlr_assert!(out.raw() >= 0);
    cvlr_assert!(out.raw() <= value);
}

/// Widened half of `bps_apply_to_wad_floor_le_value`: `value * ratio` overflows
/// `i128`, so `Wad::mul_floor` goes through the exact `I256` host path. The two
/// bounds are exact complements, so the lemma pair covers the original domain.
#[rule]
fn bps_apply_to_wad_floor_le_value_widened(e: Env, value: i128, bps: i128) {
    cvlr_assume!((0..=1_000_000 * WAD).contains(&value));
    cvlr_assume!((0..=BPS).contains(&bps));
    cvlr_assume!(value > wad_floor_native_max(bps_ratio_wad(bps)));

    let out = Bps::from(bps).apply_to_wad_floor(&e, Wad::from(value));
    cvlr_assert!(out.raw() >= 0);
    cvlr_assert!(out.raw() <= value);
}

/// Native half of `bps_apply_to_wad_floor_monotone`.
///
/// The split is stated on the larger operand `v2`: `v1 <= v2` implies
/// `v1 * ratio <= v2 * ratio`, so bounding `v2` puts both multiplications on
/// the native path.
#[rule]
fn bps_apply_to_wad_floor_monotone_native(e: Env, v1: i128, v2: i128, bps: i128) {
    cvlr_assume!((0..=1_000_000 * WAD).contains(&v1));
    cvlr_assume!((v1..=1_000_000 * WAD).contains(&v2));
    cvlr_assume!((0..=BPS).contains(&bps));
    cvlr_assume!(v2 <= wad_floor_native_max(bps_ratio_wad(bps)));

    let w1 = Bps::from(bps).apply_to_wad_floor(&e, Wad::from(v1));
    let w2 = Bps::from(bps).apply_to_wad_floor(&e, Wad::from(v2));
    cvlr_assert!(w2.raw() >= w1.raw());
}

/// Widened half of `bps_apply_to_wad_floor_monotone`: the `v2` product
/// overflows, so its multiplication is widened. The `v1` product may still be
/// native, which is the branch mix this lemma is for; the negated bound makes
/// the two lemma domains a partition of the original.
#[rule]
fn bps_apply_to_wad_floor_monotone_widened(e: Env, v1: i128, v2: i128, bps: i128) {
    cvlr_assume!((0..=1_000_000 * WAD).contains(&v1));
    cvlr_assume!((v1..=1_000_000 * WAD).contains(&v2));
    cvlr_assume!((0..=BPS).contains(&bps));
    cvlr_assume!(v2 > wad_floor_native_max(bps_ratio_wad(bps)));

    let w1 = Bps::from(bps).apply_to_wad_floor(&e, Wad::from(v1));
    let w2 = Bps::from(bps).apply_to_wad_floor(&e, Wad::from(v2));
    cvlr_assert!(w2.raw() >= w1.raw());
}

// ---------------------------------------------------------------------------
// `fp_core` identities, moved here from the controller layer on 2026-09-03.
//
// None of these rules reads controller state or calls controller code: they
// pin the behaviour of `crate::math::fp_core`, which lives in this crate. They
// were proved through the 228 KB controller artifact at entrypoint budgets
// until the migration; here they share the arithmetic artifact and its much
// smaller budgets.
// ---------------------------------------------------------------------------

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
fn rescale_upscale_lossless(e: Env) {
    let x: i128 = cvlr::nondet::nondet();
    let from: u32 = 7;
    let to: u32 = 18;

    cvlr_assume!((0..=WAD).contains(&x));

    let upscaled = rescale_half_up(&e, x, from, to);

    let factor = 10i128.pow(to - from);
    cvlr_assert!(upscaled == x * factor);
}

#[rule]
fn rescale_roundtrip(e: Env) {
    let x: i128 = cvlr::nondet::nondet();
    let low: u32 = 7;
    let high: u32 = 18;

    cvlr_assume!((0..=1_000_000_000_000_000).contains(&x));

    let upscaled = rescale_half_up(&e, x, low, high);
    let recovered = rescale_half_up(&e, upscaled, high, low);

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

/// Satisfy twin of `div_by_zero_sanity`: the same domain with the divisor gate
/// flipped (`RAY` instead of `0`) completes and returns `a`. Without it, the
/// rule above passes whether the trap is the divisor check or any other
/// unreachability in the call.
#[rule]
fn div_by_zero_sanity_fixture_completes(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    cvlr_assume!((0..=RAY).contains(&a));

    let result = mul_div_half_up(&e, a, RAY, RAY);

    cvlr_satisfy!(result == a);
}

// ---------------------------------------------------------------------------
// Anti-splitting bounds for the fixed-point primitives — the analogue of Aave
// Hub's `*Additivity` rules.
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
fn split_rescale_upscale_exact(e: Env) {
    let a1: i128 = cvlr::nondet::nondet();
    let a2: i128 = cvlr::nondet::nondet();
    let low: u32 = 7;
    let high: u32 = 18;

    cvlr_assume!((0..=WAD).contains(&a1));
    cvlr_assume!((0..=WAD).contains(&a2));

    let split = rescale_half_up(&e, a1, low, high) + rescale_half_up(&e, a2, low, high);
    let single = rescale_half_up(&e, a1 + a2, low, high);

    cvlr_assert!(split == single);
}

/// Downscaling rescale rounds half up, so splitting moves the result by at most
/// one unit of the coarser decimal scale: `|f(a1) + f(a2) − f(a1 + a2)| <= 1`.
#[rule]
fn split_rescale_downscale_bounded(e: Env) {
    let a1: i128 = cvlr::nondet::nondet();
    let a2: i128 = cvlr::nondet::nondet();
    let high: u32 = 18;
    let low: u32 = 7;

    cvlr_assume!((0..=WAD).contains(&a1));
    cvlr_assume!((0..=WAD).contains(&a2));

    let split = rescale_half_up(&e, a1, high, low) + rescale_half_up(&e, a2, high, low);
    let single = rescale_half_up(&e, a1 + a2, high, low);

    cvlr_assert!(split <= single + 1);
    cvlr_assert!(split >= single - 1);
}
