//! Chunked index accrual, protocol revenue, and bad-debt supply-index write-down
//! (floored). See `docs/reference/invariants.md` and ADR 0007.

use common::constants::SUPPLY_INDEX_FLOOR_RAW;
use common::math::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_supplier_rewards, compound_interest, protocol_fee_shares,
    supply_index_reward_shortfall, update_borrow_index, update_supply_index, MAX_COMPOUND_DELTA_MS,
};

use soroban_sdk::Env;

use crate::cache::Cache;

pub fn global_sync(env: &Env, cache: &mut Cache) {
    let total_delta_ms = cache.current_timestamp.saturating_sub(cache.last_timestamp);

    if total_delta_ms == 0 {
        return;
    }

    let mut remaining = total_delta_ms;
    while remaining > 0 {
        let chunk = remaining.min(MAX_COMPOUND_DELTA_MS);
        global_sync_step(env, cache, chunk);
        remaining = remaining.saturating_sub(chunk);
    }

    cache.last_timestamp = cache.current_timestamp;
}

fn global_sync_step(env: &Env, cache: &mut Cache, delta_ms: u64) {
    // dimensional: Token/Token -> Ray<1>; rate * TimeMs -> Ray<1> interest factor.
    let util = cache.calculate_utilization();
    let borrow_rate = calculate_borrow_rate(env, util, &cache.params);
    let interest_factor = compound_interest(env, borrow_rate, delta_ms);

    let new_borrow_index = update_borrow_index(env, cache.borrow_index, interest_factor);

    // dimensional: rewards and fee are Ray<Token(asset)> produced by debt index growth.
    let (supplier_rewards, protocol_fee) = calculate_supplier_rewards(
        env,
        &cache.params,
        cache.borrowed,
        new_borrow_index,
        cache.borrow_index,
    );

    let old_supply_index = cache.supply_index;
    let new_supply_index =
        update_supply_index(env, cache.supplied, old_supply_index, supplier_rewards);
    let supplier_shortfall = supply_index_reward_shortfall(
        env,
        cache.supplied,
        old_supply_index,
        new_supply_index,
        supplier_rewards,
    );

    cache.borrow_index = new_borrow_index;
    cache.supply_index = new_supply_index;

    // Both the configured reserve fee and reward value not distributable through
    // the virtual-offset index belong to protocol revenue. Later chunks include
    // the minted shares in supplied value and therefore in utilization.
    let protocol_reward = protocol_fee.checked_add(env, supplier_shortfall);
    add_protocol_revenue(cache, protocol_reward);
}

pub fn add_protocol_revenue(cache: &mut Cache, fee: Ray) {
    // Always mint scaled supply for the fee so `claim_revenue` can pay it out.
    if fee == Ray::ZERO {
        return;
    }
    // Overflow-safe: a floored supply index (post-wipeout) can push the raw share
    // count past i128; `protocol_fee_shares` saturates and caps to the headroom in
    // `supplied` so a bricked market never traps here.
    let fee_scaled = protocol_fee_shares(&cache.env, fee, cache.supply_index, cache.supplied);
    // Protocol revenue also counts toward total scaled supply.
    cache.revenue.checked_add_assign(&cache.env, fee_scaled);
    cache.supplied.checked_add_assign(&cache.env, fee_scaled);
}

pub fn apply_bad_debt_to_supply_index(cache: &mut Cache, bad_debt: Ray) {
    // dimensional: bad_debt and supplied * supply_index are Ray<Token(asset)>.
    let total_supplied_value = cache.supplied.mul(&cache.env, cache.supply_index);

    if total_supplied_value == Ray::ZERO {
        return;
    }

    let capped = bad_debt.min(total_supplied_value);
    let remaining = total_supplied_value.checked_sub(&cache.env, capped);

    // dimensional: remaining / total_supplied_value is Ray<1>, scaling Ray<Index(asset, supply)>.
    // Floor both steps so the writedown socializes at least the full loss (never less):
    // rounding the residual factor or the new index up would leave a dust deficit unbacked.
    let reduction_factor = remaining.div_floor(&cache.env, total_supplied_value);
    let new_supply_index = cache.supply_index.mul_floor(&cache.env, reduction_factor);

    cache.supply_index = new_supply_index.max(Ray::from(SUPPLY_INDEX_FLOOR_RAW));
}

#[cfg(test)]
#[path = "../tests/interest.rs"]
mod tests;
