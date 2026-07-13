//! Interest accrual and index math for the market cache: compounds borrow/supply
//! indexes in bounded time chunks, books protocol revenue, and socializes bad
//! debt by writing down the supply index (floored to avoid share explosion).

use common::constants::SUPPLY_INDEX_FLOOR_RAW;
use common::math::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_supplier_rewards, compound_interest, update_borrow_index,
    update_supply_index, MAX_COMPOUND_DELTA_MS,
};

use soroban_sdk::Env;

use crate::cache::Cache;

// ################## CHANGE STATE ##################

/// Accrues interest from the last pool timestamp to the current ledger timestamp.
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

/// Compounds one bounded time chunk: accrues the borrow and supply indexes and
/// books protocol revenue.
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

    let new_supply_index =
        update_supply_index(env, cache.supplied, cache.supply_index, supplier_rewards);

    cache.borrow_index = new_borrow_index;
    cache.supply_index = new_supply_index;

    // Protocol fee is added to revenue and scaled supplied; later chunks in the
    // same accrual use diluted utilization.
    add_protocol_revenue(cache, protocol_fee);
}

// ################## LOW-LEVEL HELPERS ##################

/// Adds a RAY-denominated fee as scaled protocol revenue.
pub fn add_protocol_revenue(cache: &mut Cache, fee: Ray) {
    // Zero fees, fees at/below the index floor, or fees without suppliers have no supply base.
    if fee == Ray::ZERO
        || cache.supply_index.raw() <= SUPPLY_INDEX_FLOOR_RAW
        || cache.supplied == Ray::ZERO
    {
        return;
    }
    // dimensional: Ray<Token(asset)> / Ray<Index(asset, supply)> -> Ray<Share(asset, supply)>.
    let fee_scaled = fee.div(&cache.env, cache.supply_index);
    // Protocol revenue also counts toward total scaled supply.
    cache.revenue.checked_add_assign(&cache.env, fee_scaled);
    cache.supplied.checked_add_assign(&cache.env, fee_scaled);
}

/// Socializes uncollectable debt by reducing the supply index.
pub fn apply_bad_debt_to_supply_index(cache: &mut Cache, bad_debt: Ray) {
    // dimensional: bad_debt and supplied * supply_index are Ray<Token(asset)>.
    let total_supplied_value = cache.supplied.mul(&cache.env, cache.supply_index);

    if total_supplied_value == Ray::ZERO {
        return;
    }

    let capped = bad_debt.min(total_supplied_value);
    let remaining = total_supplied_value - capped;

    // dimensional: remaining / total_supplied_value is Ray<1>, scaling Ray<Index(asset, supply)>.
    let reduction_factor = remaining.div(&cache.env, total_supplied_value);
    let new_supply_index = cache.supply_index.mul(&cache.env, reduction_factor);

    cache.supply_index = new_supply_index.max(Ray::from(SUPPLY_INDEX_FLOOR_RAW));
}

#[cfg(test)]
#[path = "../tests/interest.rs"]
mod tests;
