//! Every index movement in the pool: chunked interest accrual, protocol
//! revenue minting, supplier reward distribution, and bad-debt write-down
//! (floored). See `docs/reference/invariants.md` and ADR 0007.

use common::constants::{SUPPLY_INDEX_FLOOR_RAW, SUPPLY_INDEX_REWARD_CEILING_RAY};
use common::errors::GenericError;
use common::math::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_supplier_rewards, compound_interest, protocol_fee_shares,
    supply_index_reward_shortfall, update_borrow_index, update_supply_index, MAX_COMPOUND_DELTA_MS,
};

use soroban_sdk::{assert_with_error, Env};

use crate::cache::Cache;

/// Accrues interest up to the current ledger time. Long gaps compound in bounded
/// chunks so a single exponentiation never runs away; each chunk mints its
/// revenue shares before the next one measures utilization.
pub(crate) fn global_sync(env: &Env, cache: &mut Cache) {
    if !cache.needs_accrual() {
        return;
    }

    let mut remaining = cache.elapsed_ms();
    while remaining > 0 {
        let chunk = remaining.min(MAX_COMPOUND_DELTA_MS);
        accrue_chunk(env, cache, chunk);
        remaining = remaining.saturating_sub(chunk);
    }

    cache.mark_accrued();
}

fn accrue_chunk(env: &Env, cache: &mut Cache, delta_ms: u64) {
    // dimensional: Token/Token -> Ray<1>; rate * TimeMs -> Ray<1> interest factor.
    let util = cache.calculate_utilization();
    let borrow_rate = calculate_borrow_rate(env, util, cache.params());
    let interest_factor = compound_interest(env, borrow_rate, delta_ms);

    let new_borrow_index = update_borrow_index(env, cache.borrow_index(), interest_factor);

    // dimensional: rewards and fee are Ray<Token(asset)> produced by debt index growth.
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

    // Both the configured reserve fee and the reward value the virtual-offset
    // index cannot distribute belong to protocol revenue.
    let protocol_reward = protocol_fee.checked_add(env, supplier_shortfall);
    add_protocol_revenue(cache, protocol_reward);
}

/// Mints scaled supply shares for `fee` so `claim_revenue` can later pay it out.
pub(crate) fn add_protocol_revenue(cache: &mut Cache, fee: Ray) {
    if fee == Ray::ZERO {
        return;
    }
    // `protocol_fee_shares` saturates and caps to the headroom in `supplied`, so a
    // floor-clamped index cannot push the share count past i128 and brick accrual.
    let fee_scaled = protocol_fee_shares(cache.env(), fee, cache.supply_index(), cache.supplied());
    cache.accrue_revenue(fee_scaled);
}

/// Distributes a donated `amount` to suppliers by growing the supply index.
///
/// The virtual-offset shortfall — reward value the index cannot hand to
/// suppliers — is booked as protocol revenue instead of stranded as dead
/// reserve, so the full donation stays accounted for and backed by cash.
pub(crate) fn distribute_reward(env: &Env, cache: &mut Cache, amount: i128) {
    assert_with_error!(
        env,
        cache.supplied() != Ray::ZERO,
        GenericError::NoSuppliersToReward
    );

    // Convert only after the supplier check: `Ray::from_asset` traps on a huge
    // amount, and an empty market must report `NoSuppliersToReward` instead.
    let reward = Ray::from_asset(amount, cache.params().asset_decimals);
    let old_supply_index = cache.supply_index();
    let new_supply_index = update_supply_index(env, cache.supplied(), old_supply_index, reward);
    // Cap reward growth so repeated legs cannot pin the index at MAX.
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

/// Socializes `bad_debt` across suppliers by shrinking the supply index.
///
/// No-op when the market holds no supply: there is no one to charge, so the
/// loss is absorbed by the pool's dead reserve rather than being carried
/// forward. The index never falls below `SUPPLY_INDEX_FLOOR_RAW`.
pub(crate) fn apply_bad_debt_to_supply_index(cache: &mut Cache, bad_debt: Ray) {
    // dimensional: bad_debt and supplied * supply_index are Ray<Token(asset)>.
    let total_supplied_value = cache.supplied().mul(cache.env(), cache.supply_index());

    if total_supplied_value == Ray::ZERO {
        return;
    }

    let capped = bad_debt.min(total_supplied_value);
    let remaining = total_supplied_value.checked_sub(cache.env(), capped);

    // dimensional: remaining / total_supplied_value is Ray<1>, scaling Ray<Index(asset, supply)>.
    // Floor both steps so the writedown socializes at least the full loss (never less):
    // rounding the residual factor or the new index up would leave a dust deficit unbacked.
    let reduction_factor = remaining.div_floor(cache.env(), total_supplied_value);
    let new_supply_index = cache
        .supply_index()
        .mul_floor(cache.env(), reduction_factor);

    // When this floor clamps, the write-down is truncated: part of the loss stays
    // as supplier claim with no cash behind it. That residue is exactly what
    // `guards::require_backed_market` rejects on any later supply.
    cache.set_supply_index(new_supply_index.max(Ray::from(SUPPLY_INDEX_FLOOR_RAW)));
}

#[cfg(test)]
#[path = "../tests/interest.rs"]
mod tests;
