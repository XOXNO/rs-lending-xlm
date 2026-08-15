//! Converts between asset-unit amounts and index-scaled `Ray` values for
//! supply and borrow positions, and resolves withdrawal and repayment
//! amounts against a scaled position.

use soroban_sdk::{panic_with_error, Env};

use crate::constants::RAY;
use crate::math::fp::Ray;
use crate::math::fp_core;

/// Converts a scaled `Ray` amount to its original (unscaled) value by
/// multiplying by `index`.
pub fn scaled_to_original(env: &Env, scaled: Ray, index: Ray) -> Ray {
    scaled.mul(env, index)
}

/// Converts an asset-unit `cap` to a scaled `Ray` value using floor rounding,
/// saturating the division to `i128::MAX` instead of panicking on overflow, so
/// the cap check fails open rather than trapping an entry path. (The asset→ray
/// rescale itself panics on overflow; caps must be pre-validated to the asset's
/// decimal domain — see [`crate::validation::require_cap_within_asset_domain`].)
/// Distinct from [`calculate_scaled_supply`] / [`calculate_scaled_borrow`],
/// which panic on overflow (position accounting).
pub fn calculate_scaled_cap(env: &Env, cap: i128, decimals: u32, index: Ray) -> Ray {
    Ray::from(fp_core::mul_div_floor_saturating(
        env,
        Ray::from_asset(cap, decimals).raw(),
        RAY,
        index.raw(),
    ))
}

/// Converts an asset-unit `amount` to a scaled supply `Ray` using floor
/// rounding relative to `supply_index`.
pub fn calculate_scaled_supply(env: &Env, amount: i128, decimals: u32, supply_index: Ray) -> Ray {
    Ray::from_asset(amount, decimals).div_floor(env, supply_index)
}

/// Converts an asset-unit `amount` to a scaled supply `Ray` using ceiling
/// rounding relative to `supply_index`.
pub fn calculate_scaled_supply_ceil(
    env: &Env,
    amount: i128,
    decimals: u32,
    supply_index: Ray,
) -> Ray {
    Ray::from_asset(amount, decimals).div_ceil(env, supply_index)
}

/// Converts an asset-unit `amount` to a scaled borrow `Ray` using ceiling
/// rounding relative to `borrow_index`.
pub fn calculate_scaled_borrow(env: &Env, amount: i128, decimals: u32, borrow_index: Ray) -> Ray {
    Ray::from_asset(amount, decimals).div_ceil(env, borrow_index)
}

/// Converts an asset-unit `amount` to a scaled borrow `Ray` using floor
/// rounding relative to `borrow_index`.
pub fn calculate_scaled_borrow_floor(
    env: &Env,
    amount: i128,
    decimals: u32,
    borrow_index: Ray,
) -> Ray {
    Ray::from_asset(amount, decimals).div_floor(env, borrow_index)
}

/// Converts a scaled supply `Ray` back to an asset-unit amount, using
/// half-up rounding at `decimals` precision.
pub fn unscale_supply(env: &Env, scaled: Ray, supply_index: Ray, decimals: u32) -> i128 {
    scaled_to_original(env, scaled, supply_index).to_asset(decimals)
}

/// Converts a scaled supply `Ray` back to an asset-unit amount, using floor
/// rounding at `decimals` precision.
pub fn unscale_supply_floor(env: &Env, scaled: Ray, supply_index: Ray, decimals: u32) -> i128 {
    scaled.mul_floor(env, supply_index).to_asset_floor(decimals)
}

/// Converts a scaled borrow `Ray` back to an asset-unit amount, using
/// half-up rounding at `decimals` precision.
pub fn unscale_borrow(env: &Env, scaled: Ray, borrow_index: Ray, decimals: u32) -> i128 {
    scaled_to_original(env, scaled, borrow_index).to_asset(decimals)
}

/// Converts a scaled borrow `Ray` back to an asset-unit amount, using
/// ceiling rounding at `decimals` precision.
pub fn unscale_borrow_ceil(env: &Env, scaled: Ray, borrow_index: Ray, decimals: u32) -> i128 {
    scaled.mul_ceil(env, borrow_index).to_asset_ceil(decimals)
}

/// Converts a scaled borrow `Ray` to its original value using ceiling
/// rounding, without rescaling to asset-unit decimals.
pub fn unscale_borrow_ceil_ray(env: &Env, scaled: Ray, borrow_index: Ray) -> Ray {
    scaled.mul_ceil(env, borrow_index)
}

/// Determines the scaled and unscaled amounts to withdraw from `pos_scaled`
/// when a caller requests `amount` asset units.
///
/// If `amount` is at least the position's current (half-up-rounded) supply
/// value, treats it as a full withdrawal and returns `pos_scaled` unchanged
/// alongside the position's floor-rounded supply value. Otherwise returns the
/// ceiling-rounded scaled equivalent of `amount` alongside `amount` itself,
/// for a partial withdrawal.
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

/// Offsets same-asset supply against debt in conservative token units.
///
/// Settle amount is the overlap
/// `min(requested, floor(supply), ceil(debt))`. A side is fully closed only
/// when that overlap exhausts that side's conservative value — never because
/// a half-up display rounded up by one native unit. Partial burns stay
/// directed (`ceil` supply, `floor` debt) and are capped at the position.
///
/// Returns `(burned_supply, burned_debt, settled_tokens)`. A non-positive
/// overlap is a no-op.
pub fn resolve_net_settle(
    env: &Env,
    amount: i128,
    supply_scaled: Ray,
    debt_scaled: Ray,
    supply_index: Ray,
    borrow_index: Ray,
    decimals: u32,
) -> (Ray, Ray, i128) {
    let supply_floor = unscale_supply_floor(env, supply_scaled, supply_index, decimals);
    let debt_ceil = unscale_borrow_ceil(env, debt_scaled, borrow_index, decimals);
    let settle = amount.min(supply_floor).min(debt_ceil);
    if settle <= 0 {
        return (Ray::ZERO, Ray::ZERO, 0);
    }

    let burned_supply = if settle == supply_floor {
        supply_scaled
    } else {
        calculate_scaled_supply_ceil(env, settle, decimals, supply_index).min(supply_scaled)
    };
    let burned_debt = if settle == debt_ceil {
        debt_scaled
    } else {
        calculate_scaled_borrow_floor(env, settle, decimals, borrow_index).min(debt_scaled)
    };

    (burned_supply, burned_debt, settle)
}

/// Determines the scaled debt to **burn** and any excess repayment when a
/// caller repays `amount` asset units against `pos_scaled`.
///
/// If `amount` is at least the position's ceiling-rounded debt value, treats
/// it as a full repayment and returns `pos_scaled` (burn the whole position)
/// alongside the excess (`amount` minus that debt value). Otherwise returns
/// the floor-rounded scaled equivalent of `amount` alongside zero, for a
/// partial repayment.
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
