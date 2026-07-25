//! Index growth and the protocol's cut of it.
//!
//! Both indexes only ever grow (bad-debt write-down is the pool's job, not
//! this module's) and both are clamped. Reward distribution runs through a
//! virtual offset, so the undistributed remainder is returned separately and
//! booked as protocol revenue rather than stranded.

use soroban_sdk::Env;

use crate::constants::{MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, RAY, SUPPLY_VIRTUAL_VALUE_RAY};
use crate::math::fp::Ray;
use crate::math::fp_core;
use crate::types::MarketParams;

/// Grows the borrow index by an accrual factor, clamped at `MAX_BORROW_INDEX_RAY`.
pub fn update_borrow_index(env: &Env, old_index: Ray, interest_factor: Ray) -> Ray {
    // dimensional: Ray<Index(asset, debt)> * Ray<1> -> Ray<Index(asset, debt)>.
    let new_index = old_index.mul(env, interest_factor);
    if new_index.raw() > MAX_BORROW_INDEX_RAY {
        return Ray::from(MAX_BORROW_INDEX_RAY);
    }
    new_index
}

/// Grows the supply index to distribute `rewards_increase` across `supplied`,
/// clamped at `MAX_SUPPLY_INDEX_RAY` and never decreasing.
///
/// A virtual offset dilutes the denominator, so suppliers receive strictly less
/// than the full reward; the remainder is recovered by
/// [`supply_index_reward_shortfall`].
pub fn update_supply_index(env: &Env, supplied: Ray, old_index: Ray, rewards_increase: Ray) -> Ray {
    if supplied == Ray::ZERO || rewards_increase == Ray::ZERO {
        return old_index;
    }

    // dimensional: supplied * old_index and rewards_increase are Ray<Token(asset)>.
    let total_supplied_value = supplied.mul(env, old_index);
    // Bad-debt floor path: supplied * index can round to zero.
    if total_supplied_value == Ray::ZERO {
        return old_index;
    }
    // Virtual offset is reward-denominator only; utilization and bad-debt use the real base.
    let denom = total_supplied_value.checked_add(env, Ray::from(SUPPLY_VIRTUAL_VALUE_RAY));
    // Floor the reward ratio so index rounding cannot attribute more value to
    // suppliers than the pool actually received. Any remainder is booked as
    // protocol revenue by `supply_index_reward_shortfall`.
    let rewards_ratio = rewards_increase.div_floor(env, denom);
    // `floor(old * (1 + ratio)) == old + floor(old * ratio)`. Writing the
    // equivalent increment form makes monotonicity explicit and keeps both
    // multiplication and addition saturating at the i128 edge.
    let increment =
        fp_core::mul_div_floor_saturating(env, old_index.raw(), rewards_ratio.raw(), RAY);
    let grown = old_index.raw().saturating_add(increment);
    // Keep monotonicity structural even if an unexpected arithmetic input ever
    // reaches this helper. For every validated input this lower bound is a no-op;
    // the upper bound also preserves the existing behavior for an index above cap.
    let bounded_old = old_index.raw().min(MAX_SUPPLY_INDEX_RAY);
    Ray::from(grown.min(MAX_SUPPLY_INDEX_RAY).max(bounded_old))
}

/// Reward value that a supply-index update leaves UNDISTRIBUTED to suppliers:
/// the virtual-offset dilution plus any `MAX_SUPPLY_INDEX_RAY` clamp remainder.
/// `distributed = supplied * (new_index - old_index)`, floored by the index math,
/// so this is always `>= 0`. Booking it as protocol revenue keeps 100% of the
/// reward accounted instead of stranding it as non-extractable dead reserve,
/// while leaving the suppliers' diluted share (the dust-poisoning defense) exactly
/// as-is.
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

/// Splits interest accrued over a borrow-index step into
/// `(supplier_rewards, protocol_fee)` by the market's reserve factor.
pub fn calculate_supplier_rewards(
    env: &Env,
    params: &MarketParams,
    borrowed: Ray,
    new_borrow_index: Ray,
    old_borrow_index: Ray,
) -> (Ray, Ray) {
    // dimensional: borrowed is Ray<Share(asset, debt)>; indexes lift it to Ray<Token(asset)>.
    let old_total_debt = borrowed.mul(env, old_borrow_index);
    let new_total_debt = borrowed.mul(env, new_borrow_index);

    let accrued_interest = new_total_debt.checked_sub(env, old_total_debt);

    let protocol_fee = params.reserve_factor.apply_to_ray(env, accrued_interest);
    let supplier_rewards = accrued_interest.checked_sub(env, protocol_fee);

    (supplier_rewards, protocol_fee)
}

/// Scales a protocol `fee` into supply shares without over-crediting revenue.
/// Floor rounding keeps the minted claim at or below the fee value. At a floored
/// supply index (post-wipeout) the raw share count can exceed `i128`, so the
/// conversion saturates and is capped to the headroom left in `supplied` — accrual
/// and the simulate view can never trap on a bricked market.
pub fn protocol_fee_shares(env: &Env, fee: Ray, supply_index: Ray, supplied: Ray) -> Ray {
    let raw = fp_core::mul_div_floor_saturating(env, fee.raw(), RAY, supply_index.raw());
    // `supplied` is non-negative by construction, so this cannot overflow; use
    // the saturating form anyway rather than being the file's one bare subtraction.
    let headroom = i128::MAX.saturating_sub(supplied.raw());
    Ray::from(raw.min(headroom))
}

#[cfg(test)]
#[path = "../../tests/rates/index.rs"]
mod tests;
