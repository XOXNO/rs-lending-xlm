use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use crate::constants::{BPS, RAY, WAD};
use crate::fp::{Bps, Ray, Wad};

#[rule]
fn ray_mul_identity(e: Env, amount: i128) {
    cvlr_assume!((0..=10 * RAY).contains(&amount));

    let value = Ray::from_raw(amount);
    cvlr_assert!(value.mul(&e, Ray::ONE).raw() == amount);
    cvlr_assert!(Ray::ONE.mul(&e, value).raw() == amount);
}

#[rule]
fn ray_div_floor_never_exceeds_half_up(e: Env, amount: i128, divisor: i128) {
    cvlr_assume!((0..=10 * RAY).contains(&amount));
    cvlr_assume!((1..=10 * RAY).contains(&divisor));

    let half_up = Ray::from_raw(amount).div(&e, Ray::from_raw(divisor));
    let floor = Ray::from_raw(amount).div_floor(&e, Ray::from_raw(divisor));
    cvlr_assert!(floor.raw() <= half_up.raw());
}

#[rule]
fn ray_asset_roundtrip_preserves_7_decimal_amount(amount: i128) {
    cvlr_assume!((0..=1_000_000_000_000_000i128).contains(&amount));

    let ray = Ray::from_asset(amount, 7);
    cvlr_assert!(ray.to_asset(7) == amount);
}

#[rule]
fn wad_token_roundtrip_preserves_7_decimal_amount(amount: i128) {
    cvlr_assume!((0..=1_000_000_000_000_000i128).contains(&amount));

    let wad = Wad::from_token(amount, 7);
    cvlr_assert!(wad.to_token(7) == amount);
}

#[rule]
fn wad_to_ray_preserves_one() {
    cvlr_assert!(Wad::ONE.to_ray().raw() == RAY);
}

#[rule]
fn bps_apply_to_ray_is_bounded(e: Env, value: i128, bps: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&value));
    cvlr_assume!((0..=BPS).contains(&bps));

    let out = Bps::from_raw(bps).apply_to_ray(&e, Ray::from_raw(value));
    cvlr_assert!(out.raw() >= 0);
    cvlr_assert!(out.raw() <= value);
}

#[rule]
fn bps_one_is_identity_on_wad(e: Env, value: i128) {
    cvlr_assume!((0..=100 * WAD).contains(&value));

    let out = Bps::ONE.apply_to_wad(&e, Wad::from_raw(value));
    cvlr_assert!(out.raw() == value);
}

#[rule]
fn common_math_reachability(e: Env, amount: i128) {
    cvlr_assume!(amount > 0 && amount <= RAY);
    let out = Ray::from_raw(amount).mul(&e, Ray::ONE);
    cvlr_satisfy!(out.raw() > 0);
}
