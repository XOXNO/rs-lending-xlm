use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::Env;

use crate::constants::WAD;
use common::constants::RAY;
use common::math::fp::{Ray, Wad};

#[rule]
fn position_value_monotone_in_scaled(e: Env, s1: i128, s2: i128, index: i128, price: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&s1));
    cvlr_assume!((s1..=100 * RAY).contains(&s2));
    cvlr_assume!((RAY..=10 * RAY).contains(&index));
    cvlr_assume!((1..=1_000_000 * WAD).contains(&price));

    let v1 = crate::risk::position_value(&e, Ray::from(s1), Ray::from(index), Wad::from(price));
    let v2 = crate::risk::position_value(&e, Ray::from(s2), Ray::from(index), Wad::from(price));
    cvlr_assert!(v2.raw() >= v1.raw());
}

#[rule]
fn position_value_ceil_ge_floor(e: Env, scaled: i128, index: i128, price: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&scaled));
    cvlr_assume!((RAY..=10 * RAY).contains(&index));
    cvlr_assume!((1..=1_000_000 * WAD).contains(&price));

    let ceil =
        crate::risk::position_value_ceil(&e, Ray::from(scaled), Ray::from(index), Wad::from(price));
    let floor = crate::risk::position_value_floor(
        &e,
        Ray::from(scaled),
        Ray::from(index),
        Wad::from(price),
    );
    cvlr_assert!(ceil.raw() >= floor.raw());
}
