use soroban_sdk::Env;

use crate::constants::{MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, RAY, SUPPLY_VIRTUAL_VALUE_RAY};
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

/// Grows the supply index so `rewards_increase` accrues to suppliers, diluted by
/// the `SUPPLY_VIRTUAL_VALUE_RAY` offset that keeps a dust-sized market from
/// inflating the index without bound.
///
/// Both roundings are **floor** (`div_floor`, then `mul_div_floor_saturating`),
/// so the index is never moved further than the reward actually pays for; the
/// residue is recovered by [`supply_index_reward_shortfall`] and booked as
/// protocol revenue. The result is clamped into
/// `[min(old_index, MAX_SUPPLY_INDEX_RAY), MAX_SUPPLY_INDEX_RAY]`.
///
/// # Preconditions
///
/// `old_index <= MAX_SUPPLY_INDEX_RAY`. Above the cap the clamp moves the index
/// *down*, which [`supply_index_reward_shortfall`] rejects. The pool upholds
/// this: markets open at `RAY`, every write either passes through this clamp or
/// through the bad-debt path, which only reduces the index.
///
/// Panics with [`GenericError::MathOverflow`](crate::errors::GenericError) if
/// `supplied * old_index / RAY` does not fit in `i128` — that is, if the
/// market's total value exceeds the `i128` ceiling.
pub fn update_supply_index(env: &Env, supplied: Ray, old_index: Ray, rewards_increase: Ray) -> Ray {
    if supplied == Ray::ZERO || rewards_increase == Ray::ZERO {
        return old_index;
    }

    let total_supplied_value = supplied.mul(env, old_index);

    if total_supplied_value == Ray::ZERO {
        return old_index;
    }

    let denom = total_supplied_value.checked_add(env, Ray::from(SUPPLY_VIRTUAL_VALUE_RAY));

    let rewards_ratio = rewards_increase.div_floor(env, denom);

    let increment =
        fp_core::mul_div_floor_saturating(env, old_index.raw(), rewards_ratio.raw(), RAY);
    let grown = old_index.raw().saturating_add(increment);

    let bounded_old = old_index.raw().min(MAX_SUPPLY_INDEX_RAY);
    Ray::from(grown.min(MAX_SUPPLY_INDEX_RAY).max(bounded_old))
}

/// Returns the part of `rewards_increase` that the index move did *not* hand to
/// suppliers, so callers can book it as protocol revenue instead of letting it
/// vanish. `distributed + shortfall == rewards_increase` exactly.
///
/// `new_index` must come from [`update_supply_index`] called with the same
/// `supplied`, `old_index` and `rewards_increase`.
///
/// The measurement rounds **half-up** on both legs while `update_supply_index`
/// derived the index by flooring twice, so the two disagree by up to 1 ulp. That
/// is safe: writing `V = round_half_up(supplied * old_index / RAY)` and
/// `inc = floor(old_index * floor(R * RAY / (V + RAY)) / RAY)`, we have
/// `supplied * inc / RAY <= R * (V + 0.5) / (V + RAY) < R`, so
/// `floor(supplied * inc / RAY) <= R - 1`, and the half-up pair can add at most
/// one more unit — leaving `distributed <= R`. The slack is `~R * RAY /
/// (V + RAY)` and it does reach zero, so the bound is tight but never violated.
/// Pinned by the sweeps in `common/tests/rates/index.rs`.
///
/// # Preconditions
///
/// `new_index >= old_index`, which holds for any `old_index <=
/// MAX_SUPPLY_INDEX_RAY`. Otherwise the `MAX_SUPPLY_INDEX_RAY` clamp inside
/// `update_supply_index` moves the index down, `distributed` goes negative and
/// this panics with
/// [`GenericError::MathOverflow`](crate::errors::GenericError).
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
