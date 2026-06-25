use common::constants::SUPPLY_INDEX_FLOOR_RAW;
use common::math::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_supplier_rewards, compound_interest, update_borrow_index,
    update_supply_index, MAX_COMPOUND_DELTA_MS,
};
use soroban_sdk::Env;

use crate::cache::Cache;

/// Accrues interest from the last pool timestamp to the current ledger timestamp.
pub fn global_sync(env: &Env, cache: &mut Cache) {
    let total_delta_ms = cache.current_timestamp.saturating_sub(cache.last_timestamp);

    if total_delta_ms == 0 {
        return;
    }

    let mut remaining = total_delta_ms;
    while remaining > 0 {
        let chunk = core::cmp::min(remaining, MAX_COMPOUND_DELTA_MS);
        global_sync_step(env, cache, chunk);
        remaining = remaining.saturating_sub(chunk);
    }

    cache.last_timestamp = cache.current_timestamp;
}

fn global_sync_step(env: &Env, cache: &mut Cache, delta_ms: u64) {
    let util = cache.calculate_utilization();
    let borrow_rate = calculate_borrow_rate(env, util, &cache.params);
    let interest_factor = compound_interest(env, borrow_rate, delta_ms);

    let new_borrow_index = update_borrow_index(env, cache.borrow_index, interest_factor);

    let (supplier_rewards, protocol_fee) = calculate_supplier_rewards(
        env,
        &cache.params,
        cache.borrowed,
        new_borrow_index,
        cache.borrow_index,
    );

    let new_supply_index =
        update_supply_index(env, cache.supplied, cache.supply_index, supplier_rewards);

    cache.borrow_index = new_borrow_index;
    cache.supply_index = new_supply_index;

    // Protocol fee is added to revenue and scaled supplied; later chunks in the
    // same accrual use diluted utilization.
    add_protocol_revenue_ray(cache, protocol_fee);
}

/// Adds a RAY-denominated fee as scaled protocol revenue.
pub fn add_protocol_revenue_ray(cache: &mut Cache, fee: Ray) {
    if fee == Ray::ZERO {
        return;
    }
    if cache.supply_index.raw() <= SUPPLY_INDEX_FLOOR_RAW {
        return;
    }
    // Fees on an empty pool are dropped; there are no suppliers to dilute.
    if cache.supplied == Ray::ZERO {
        return;
    }
    let fee_scaled = fee.div(&cache.env, cache.supply_index);
    cache.revenue.checked_add_assign(&cache.env, fee_scaled);
    cache.supplied.checked_add_assign(&cache.env, fee_scaled);
}

/// Socializes uncollectable debt by reducing the supply index.
pub fn apply_bad_debt_to_supply_index(cache: &mut Cache, bad_debt: Ray) {
    let total_supplied_value = cache.supplied.mul(&cache.env, cache.supply_index);

    if total_supplied_value == Ray::ZERO {
        return;
    }

    let capped = if bad_debt > total_supplied_value {
        total_supplied_value
    } else {
        bad_debt
    };
    let remaining = total_supplied_value - capped;

    let reduction_factor = remaining.div(&cache.env, total_supplied_value);
    let new_supply_index = cache.supply_index.mul(&cache.env, reduction_factor);

    let floor_index = Ray::from(SUPPLY_INDEX_FLOOR_RAW);

    cache.supply_index = if new_supply_index < floor_index {
        floor_index
    } else {
        new_supply_index
    };
}

#[cfg(test)]
#[path = "../tests/interest.rs"]
mod tests;
