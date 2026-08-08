use soroban_sdk::Env;

use crate::math::fp::{Ray, Wad};

/// Half-up position USD value: `(scaled * index).to_wad() * price`.
///
/// Stays in ray space through the index mul, then rescales to WAD before the
/// price mul — preferred over unscale-to-asset then `PriceFeed::usd_value_wad`
/// when asset decimals would only be re-upscaled to WAD.
#[inline]
pub fn position_value(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul(env, index);
    let actual_wad = actual.to_wad();
    actual_wad.mul(env, price)
}

/// Floor position USD value (borrow/LTV gates that must not overstate collateral).
#[inline]
pub fn position_value_floor(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul_floor(env, index);
    let actual_wad = actual.to_wad_floor();
    actual_wad.mul_floor(env, price)
}

/// Ceil position USD value (debt totals that must not understate liability).
#[inline]
pub fn position_value_ceil(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul_ceil(env, index);
    let actual_wad = actual.to_wad_ceil();
    actual_wad.mul_ceil(env, price)
}

#[cfg(test)]
#[path = "../../tests/rates/value.rs"]
mod tests;
