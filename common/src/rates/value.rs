//! Converts a scaled position balance (ray-precision units, e.g. principal
//! divided by an index) into a USD value expressed in WAD, given the
//! applicable index and asset price. Each function performs the same
//! sequence — multiply by the index, rescale from ray to WAD, multiply by
//! the price — using a different rounding direction at every step.

use soroban_sdk::Env;

use crate::math::fp::{Ray, Wad};

/// Computes the USD value of a scaled position using half-up rounding at
/// each step: multiplies `scaled` by `index` in ray precision, rescales the
/// product to WAD, then multiplies by `price`.
#[inline]
pub fn position_value(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul(env, index);
    let actual_wad = actual.to_wad();
    actual_wad.mul(env, price)
}

/// Computes the USD value of a scaled position, rounding down at each step:
/// multiplies `scaled` by `index`, rescales the product to WAD, then
/// multiplies by `price`, using floor rounding throughout.
#[inline]
pub fn position_value_floor(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul_floor(env, index);
    let actual_wad = actual.to_wad_floor();
    actual_wad.mul_floor(env, price)
}

/// Computes the USD value of a scaled position, rounding up at each step:
/// multiplies `scaled` by `index`, rescales the product to WAD, then
/// multiplies by `price`, using ceiling rounding throughout.
#[inline]
pub fn position_value_ceil(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul_ceil(env, index);
    let actual_wad = actual.to_wad_ceil();
    actual_wad.mul_ceil(env, price)
}

#[cfg(test)]
#[path = "../../tests/rates/value.rs"]
mod tests;
