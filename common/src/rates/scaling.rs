use soroban_sdk::{panic_with_error, Env};

use crate::constants::RAY;
use crate::math::fp::Ray;
use crate::math::fp_core;

pub fn scaled_to_original(env: &Env, scaled: Ray, index: Ray) -> Ray {
    scaled.mul(env, index)
}

/// Asset-unit amount → scaled ray with floor rounding, saturating on overflow.
///
/// Used for spoke supply/borrow **caps** where overflow must not panic an entry
/// path: the scaled cap becomes `i128::MAX`, so the cap check fails open rather
/// than trapping. Distinct from [`calculate_scaled_supply`] /
/// [`calculate_scaled_borrow`], which panic on overflow (position accounting).
pub fn calculate_scaled_cap(env: &Env, cap: i128, decimals: u32, index: Ray) -> Ray {
    Ray::from(fp_core::mul_div_floor_saturating(
        env,
        Ray::from_asset(cap, decimals).raw(),
        RAY,
        index.raw(),
    ))
}

pub fn calculate_scaled_supply(env: &Env, amount: i128, decimals: u32, supply_index: Ray) -> Ray {
    Ray::from_asset(amount, decimals).div_floor(env, supply_index)
}

pub fn calculate_scaled_supply_ceil(
    env: &Env,
    amount: i128,
    decimals: u32,
    supply_index: Ray,
) -> Ray {
    Ray::from_asset(amount, decimals).div_ceil(env, supply_index)
}

pub fn calculate_scaled_borrow(env: &Env, amount: i128, decimals: u32, borrow_index: Ray) -> Ray {
    Ray::from_asset(amount, decimals).div_ceil(env, borrow_index)
}

pub fn calculate_scaled_borrow_floor(
    env: &Env,
    amount: i128,
    decimals: u32,
    borrow_index: Ray,
) -> Ray {
    Ray::from_asset(amount, decimals).div_floor(env, borrow_index)
}

pub fn unscale_supply(env: &Env, scaled: Ray, supply_index: Ray, decimals: u32) -> i128 {
    scaled_to_original(env, scaled, supply_index).to_asset(decimals)
}

pub fn unscale_supply_floor(env: &Env, scaled: Ray, supply_index: Ray, decimals: u32) -> i128 {
    scaled.mul_floor(env, supply_index).to_asset_floor(decimals)
}

pub fn unscale_borrow(env: &Env, scaled: Ray, borrow_index: Ray, decimals: u32) -> i128 {
    scaled_to_original(env, scaled, borrow_index).to_asset(decimals)
}

pub fn unscale_borrow_ceil(env: &Env, scaled: Ray, borrow_index: Ray, decimals: u32) -> i128 {
    scaled.mul_ceil(env, borrow_index).to_asset_ceil(decimals)
}

pub fn unscale_borrow_ceil_ray(env: &Env, scaled: Ray, borrow_index: Ray) -> Ray {
    scaled.mul_ceil(env, borrow_index)
}

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
