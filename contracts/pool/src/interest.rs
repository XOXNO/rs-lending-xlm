use core::num::NonZeroU64;

use common::constants::{SUPPLY_INDEX_FLOOR_RAW, SUPPLY_INDEX_REWARD_CEILING_RAY};
use common::errors::GenericError;
use common::math::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_supplier_rewards, compound_interest, protocol_fee_shares,
    supply_index_reward_shortfall, update_borrow_index, update_supply_index, MAX_COMPOUND_DELTA_MS,
};

use soroban_sdk::{assert_with_error, Env};

use crate::cache::Cache;

pub(crate) fn global_sync(env: &Env, cache: &mut Cache) {
    if !cache.needs_accrual() {
        return;
    }

    let mut remaining = cache.elapsed_ms();
    while let Some(nonzero) = NonZeroU64::new(remaining) {
        let chunk = nonzero.get().min(MAX_COMPOUND_DELTA_MS);
        accrue_chunk(env, cache, chunk);
        remaining = remaining.saturating_sub(chunk);
    }

    cache.mark_accrued();
}

fn accrue_chunk(env: &Env, cache: &mut Cache, delta_ms: u64) {
    let util = cache.calculate_utilization();
    let borrow_rate = calculate_borrow_rate(env, util, cache.params());
    let interest_factor = compound_interest(env, borrow_rate, delta_ms);

    let new_borrow_index = update_borrow_index(env, cache.borrow_index(), interest_factor);

    let (supplier_rewards, protocol_fee) = calculate_supplier_rewards(
        env,
        cache.params(),
        cache.borrowed(),
        new_borrow_index,
        cache.borrow_index(),
    );

    let old_supply_index = cache.supply_index();
    let new_supply_index =
        update_supply_index(env, cache.supplied(), old_supply_index, supplier_rewards);
    let supplier_shortfall = supply_index_reward_shortfall(
        env,
        cache.supplied(),
        old_supply_index,
        new_supply_index,
        supplier_rewards,
    );

    cache.set_borrow_index(new_borrow_index);
    cache.set_supply_index(new_supply_index);

    let protocol_reward = protocol_fee.checked_add(env, supplier_shortfall);
    add_protocol_revenue(cache, protocol_reward);
}

pub(crate) fn add_protocol_revenue(cache: &mut Cache, fee: Ray) {
    if fee == Ray::ZERO {
        return;
    }

    let fee_scaled = protocol_fee_shares(cache.env(), fee, cache.supply_index(), cache.supplied());
    cache.accrue_revenue(fee_scaled);
}

pub(crate) fn distribute_reward(env: &Env, cache: &mut Cache, amount: i128) {
    assert_with_error!(
        env,
        cache.supplied() != Ray::ZERO,
        GenericError::NoSuppliersToReward
    );

    let reward = Ray::from_asset(amount, cache.params().asset_decimals);
    let old_supply_index = cache.supply_index();
    let new_supply_index = update_supply_index(env, cache.supplied(), old_supply_index, reward);
    assert_with_error!(
        env,
        new_supply_index.raw() <= SUPPLY_INDEX_REWARD_CEILING_RAY,
        GenericError::SupplyIndexRewardCeiling
    );
    cache.set_supply_index(new_supply_index);

    let offset_shortfall = supply_index_reward_shortfall(
        env,
        cache.supplied(),
        old_supply_index,
        new_supply_index,
        reward,
    );
    add_protocol_revenue(cache, offset_shortfall);
}

pub(crate) fn apply_bad_debt_to_supply_index(cache: &mut Cache, bad_debt: Ray) {
    let total_supplied_value = cache.supplied().mul(cache.env(), cache.supply_index());

    if total_supplied_value == Ray::ZERO {
        return;
    }

    let capped = bad_debt.min(total_supplied_value);
    let remaining = total_supplied_value.checked_sub(cache.env(), capped);

    let reduction_factor = remaining.div_floor(cache.env(), total_supplied_value);
    let new_supply_index = cache
        .supply_index()
        .mul_floor(cache.env(), reduction_factor);

    cache.set_supply_index(new_supply_index.max(Ray::from(SUPPLY_INDEX_FLOOR_RAW)));
}

#[cfg(test)]
#[path = "../tests/interest.rs"]
mod tests;
