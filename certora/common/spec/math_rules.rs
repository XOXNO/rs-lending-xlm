use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use crate::constants::{BPS, RAY, WAD};
use crate::math::fp::{Bps, Ray, Wad};

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
