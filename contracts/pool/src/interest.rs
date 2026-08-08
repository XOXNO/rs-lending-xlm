//! Interest accrual, protocol fee booking, and bad-debt socialization.
//!
//! Time is advanced in chunks of at most [`MAX_COMPOUND_DELTA_MS`] so compound
//! interest stays within the fixed-point rate engine's safe domain.

use core::num::NonZeroU64;

use common::constants::SUPPLY_INDEX_FLOOR_RAW;
use common::math::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_supplier_rewards, compound_interest, protocol_fee_shares,
    supply_index_reward_shortfall, update_borrow_index, update_supply_index, MAX_COMPOUND_DELTA_MS,
};

use soroban_sdk::Env;

use crate::cache::Cache;

/// Accrue borrow/supply indexes from `last_timestamp` to the cache's current time.
///
/// No-op when no time has elapsed. Splits long gaps into max-sized compound
/// windows, then sets `last_timestamp` to `current_timestamp`.
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

/// Apply one compound step of `delta_ms` to indexes and protocol revenue.
///
/// 1. Borrow rate from current utilization and params.
/// 2. Compound borrow index.
/// 3. Split interest into supplier rewards vs protocol fee (reserve factor).
/// 4. Grow supply index; residual that cannot raise the index becomes protocol fee.
/// 5. Book protocol fee as scaled revenue shares via [`add_protocol_revenue`].
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

/// Convert a RAY-denominated fee into scaled supply shares and mint them as revenue.
///
/// Increases both `revenue` and total `supplied` so revenue participates in the
/// supply index until claimed. No-op for zero fee.
pub(crate) fn add_protocol_revenue(cache: &mut Cache, fee: Ray) {
    if fee == Ray::ZERO {
        return;
    }

    let fee_scaled = protocol_fee_shares(cache.env(), fee, cache.supply_index(), cache.supplied());
    cache.accrue_revenue(fee_scaled);
}

/// Socialize `bad_debt` by reducing the supply index (capped at total supply value).
///
/// Used when seizing unpaid debt: remaining supplier claims shrink pro-rata.
/// Index is floored at [`SUPPLY_INDEX_FLOOR_RAW`] to avoid a zero index.
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
