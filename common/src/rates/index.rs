use soroban_sdk::Env;

use crate::constants::{MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, RAY};
use crate::math::fp::Ray;
use crate::math::fp_core;
use crate::types::MarketParams;

pub fn update_borrow_index(env: &Env, old_index: Ray, interest_factor: Ray) -> Ray {
    let new_index = old_index.mul(env, interest_factor);
    if new_index.raw() > MAX_BORROW_INDEX_RAY {
        return Ray::from(MAX_BORROW_INDEX_RAY);
    }
    new_index
}

pub fn update_supply_index(env: &Env, supplied: Ray, old_index: Ray, rewards_increase: Ray) -> Ray {
    if supplied == Ray::ZERO || rewards_increase == Ray::ZERO {
        return old_index;
    }

    let total_supplied_value = supplied.mul(env, old_index);

    if total_supplied_value == Ray::ZERO {
        return old_index;
    }

    let new_value = total_supplied_value.checked_add(env, rewards_increase);
    let grown = fp_core::mul_div_floor_saturating(env, new_value.raw(), RAY, supplied.raw());

    let bounded_old = old_index.raw().min(MAX_SUPPLY_INDEX_RAY);
    Ray::from(grown.min(MAX_SUPPLY_INDEX_RAY).max(bounded_old))
}

pub fn supply_index_reward_shortfall(
    env: &Env,
    supplied: Ray,
    old_index: Ray,
    new_index: Ray,
    rewards_increase: Ray,
) -> Ray {
    let distributed = supplied
        .mul(env, new_index)
        .checked_sub(env, supplied.mul(env, old_index));
    rewards_increase.checked_sub(env, distributed)
}

pub fn calculate_supplier_rewards(
    env: &Env,
    params: &MarketParams,
    borrowed: Ray,
    new_borrow_index: Ray,
    old_borrow_index: Ray,
) -> (Ray, Ray) {
    let old_total_debt = borrowed.mul(env, old_borrow_index);
    let new_total_debt = borrowed.mul(env, new_borrow_index);

    let accrued_interest = new_total_debt.checked_sub(env, old_total_debt);

    let protocol_fee = params.reserve_factor.apply_to_ray(env, accrued_interest);
    let supplier_rewards = accrued_interest.checked_sub(env, protocol_fee);

    (supplier_rewards, protocol_fee)
}

pub fn protocol_fee_shares(env: &Env, fee: Ray, supply_index: Ray, supplied: Ray) -> Ray {
    let raw = fp_core::mul_div_floor_saturating(env, fee.raw(), RAY, supply_index.raw());

    let headroom = i128::MAX.saturating_sub(supplied.raw());
    Ray::from(raw.min(headroom))
}

#[cfg(test)]
#[path = "../../tests/rates/index.rs"]
mod tests;
