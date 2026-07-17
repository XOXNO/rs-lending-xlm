//! Pure HF-layer lemmas over `risk` helpers (no entry points).
//! Weighted-collateral bps lemmas live in common/spec/math_rules.rs (math-hard).
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use crate::constants::WAD;
use common::constants::{BPS, RAY};
use common::math::fp::{Bps, Ray, Wad};

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

/// Debt ceil valuation >= collateral floor valuation of the same position.
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

/// HF uses div_floor: never overstates safety vs half-up rounding.
#[rule]
fn hf_division_rounds_against_borrower(e: Env, weighted: i128, debt: i128) {
    cvlr_assume!((0..=1_000_000 * WAD).contains(&weighted));
    cvlr_assume!((1..=1_000_000 * WAD).contains(&debt));

    let floor = Wad::from(weighted).div_floor(&e, Wad::from(debt));
    let half_up = Wad::from(weighted).div(&e, Wad::from(debt));
    cvlr_assert!(floor.raw() <= half_up.raw());
}

#[rule]
fn hf_lemmas_reachability(e: Env, value: i128) {
    cvlr_assume!(value > 0 && value <= WAD);
    let w = crate::risk::weighted_collateral(&e, Wad::from(value), Bps::from(BPS));
    cvlr_satisfy!(w.raw() > 0);
}
