//! Scaled-share conversions and settlement resolution.
//!
//! Every rounding direction here is chosen to favour the pool over the
//! counterparty: floor what is credited or paid out, ceil what is owed or
//! burned. `scaled_to_original`, `calculate_scaled_borrow`, `unscale_borrow_ceil`
//! and `resolve_withdrawal` are also called by the controller — its dust and
//! limit gates must agree with the pool exactly or the two diverge.

use soroban_sdk::{panic_with_error, Env};

use crate::math::fp::Ray;

/// Lifts a scaled share balance to its token value at `index`, half-up.
pub fn scaled_to_original(env: &Env, scaled: Ray, index: Ray) -> Ray {
    // dimensional: Ray<Share(asset, side)> * Ray<Index(asset, side)> -> Ray<Token(asset)>.
    scaled.mul(env, index)
}

/// Mint-path supply scaling: floor so minted shares never exceed cash in.
pub fn calculate_scaled_supply(env: &Env, amount: i128, decimals: u32, supply_index: Ray) -> Ray {
    Ray::from_asset(amount, decimals).div_floor(env, supply_index)
}

/// Burn-path supply scaling: ceil so cash out never exceeds burned value.
pub fn calculate_scaled_supply_ceil(
    env: &Env,
    amount: i128,
    decimals: u32,
    supply_index: Ray,
) -> Ray {
    Ray::from_asset(amount, decimals).div_ceil(env, supply_index)
}

/// Borrow-path debt scaling: ceil so recorded debt covers cash out.
pub fn calculate_scaled_borrow(env: &Env, amount: i128, decimals: u32, borrow_index: Ray) -> Ray {
    Ray::from_asset(amount, decimals).div_ceil(env, borrow_index)
}

/// Repay-path debt scaling: floor so debt burned never exceeds cash in.
pub fn calculate_scaled_borrow_floor(
    env: &Env,
    amount: i128,
    decimals: u32,
    borrow_index: Ray,
) -> Ray {
    Ray::from_asset(amount, decimals).div_floor(env, borrow_index)
}

/// Half-up supply unscale (full-close threshold / display).
pub fn unscale_supply(env: &Env, scaled: Ray, supply_index: Ray, decimals: u32) -> i128 {
    scaled_to_original(env, scaled, supply_index).to_asset(decimals)
}

/// Floor supply unscale (payouts, fee clamps, revenue claims).
pub fn unscale_supply_floor(env: &Env, scaled: Ray, supply_index: Ray, decimals: u32) -> i128 {
    scaled.mul_floor(env, supply_index).to_asset_floor(decimals)
}

/// Half-up borrow unscale.
pub fn unscale_borrow(env: &Env, scaled: Ray, borrow_index: Ray, decimals: u32) -> i128 {
    scaled_to_original(env, scaled, borrow_index).to_asset(decimals)
}

/// Ceil borrow unscale (debt owed / full-close amount).
pub fn unscale_borrow_ceil(env: &Env, scaled: Ray, borrow_index: Ray, decimals: u32) -> i128 {
    scaled.mul_ceil(env, borrow_index).to_asset_ceil(decimals)
}

/// Ceil borrow unscale kept in RAY (bad-debt write-down).
pub fn unscale_borrow_ceil_ray(env: &Env, scaled: Ray, borrow_index: Ray) -> Ray {
    scaled.mul_ceil(env, borrow_index)
}

/// Full-close when request ≥ half-up actual: burns all shares, pays floor gross.
/// Controller dust gates MUST mirror this rule or position map and pool diverge.
pub fn resolve_withdrawal(
    env: &Env,
    amount: i128,
    pos_scaled: Ray,
    supply_index: Ray,
    decimals: u32,
) -> (Ray, i128) {
    let current_supply_actual = unscale_supply(env, pos_scaled, supply_index, decimals);
    let current_supply_floor = unscale_supply_floor(env, pos_scaled, supply_index, decimals);
    if amount >= current_supply_actual {
        return (pos_scaled, current_supply_floor);
    }
    (
        calculate_scaled_supply_ceil(env, amount, decimals, supply_index),
        amount,
    )
}

/// Full debt close when payment ≥ ceil owed; else floor-scale the repay.
/// Returns `(scaled_burned, overpayment)`.
pub fn resolve_repay(
    env: &Env,
    amount: i128,
    pos_scaled: Ray,
    borrow_index: Ray,
    decimals: u32,
) -> (Ray, i128) {
    let current_debt_ceil = unscale_borrow_ceil(env, pos_scaled, borrow_index, decimals);
    if amount >= current_debt_ceil {
        (
            pos_scaled,
            amount.checked_sub(current_debt_ceil).unwrap_or_else(|| {
                panic_with_error!(env, crate::errors::GenericError::MathOverflow)
            }),
        )
    } else {
        (
            calculate_scaled_borrow_floor(env, amount, decimals, borrow_index),
            0,
        )
    }
}

#[cfg(test)]
#[path = "../../tests/rates/scaling.rs"]
mod tests;
