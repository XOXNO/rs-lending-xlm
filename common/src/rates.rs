//! Kinked borrow-rate curve, Taylor compound factor, and read-path index simulation.
//!
//! Rates and indexes are RAY (`1e27`); reserve factor is BPS. Accrual chunks at
//! most one year (`MAX_COMPOUND_DELTA_MS`). See `docs/reference/invariants.md`.

use soroban_sdk::{panic_with_error, Env, I256};

use crate::constants::{
    BPS, MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, MILLISECONDS_PER_YEAR, RAY,
    SUPPLY_VIRTUAL_VALUE_RAY,
};
use crate::math::fp::{Bps, Ray};
use crate::math::fp_core;
use crate::types::{MarketParams, PoolState, PoolSyncData};

/// Max compound-interest chunk (one year in ms).
pub const MAX_COMPOUND_DELTA_MS: u64 = MILLISECONDS_PER_YEAR;

pub fn calculate_borrow_rate(env: &Env, utilization: Ray, params: &MarketParams) -> Ray {
    // dimensional: utilization is Ray<1>; model slopes are Ray<RatePerYear>.
    let annual_rate = if utilization < params.mid_utilization {
        let contribution = utilization
            .mul(env, params.slope1)
            .div(env, params.mid_utilization);
        params.base_borrow_rate.checked_add(env, contribution)
    } else if utilization < params.optimal_utilization {
        let excess = utilization.checked_sub(env, params.mid_utilization);
        let range = params
            .optimal_utilization
            .checked_sub(env, params.mid_utilization);
        let contribution = excess.mul(env, params.slope2).div(env, range);
        params
            .base_borrow_rate
            .checked_add(env, params.slope1)
            .checked_add(env, contribution)
    } else {
        let base_rate = params
            .base_borrow_rate
            .checked_add(env, params.slope1)
            .checked_add(env, params.slope2);
        let excess = utilization.checked_sub(env, params.optimal_utilization);
        let range = Ray::ONE.checked_sub(env, params.optimal_utilization);
        let contribution = excess.mul(env, params.slope3).div(env, range);
        base_rate.checked_add(env, contribution)
    };

    let capped = if annual_rate > params.max_borrow_rate {
        params.max_borrow_rate
    } else {
        annual_rate
    };
    capped.div_by_int(MILLISECONDS_PER_YEAR as i128)
}

pub fn calculate_deposit_rate(
    env: &Env,
    utilization: Ray,
    borrow_rate: Ray,
    reserve_factor: Bps,
) -> Ray {
    if utilization == Ray::ZERO {
        return Ray::ZERO;
    }

    // Invalid reserve_factor → zero supplier rate (upstream also rejects ≥ BPS).
    if !(0..BPS).contains(&reserve_factor.raw()) {
        return Ray::ZERO;
    }

    let rate_x_util = utilization.mul(env, borrow_rate);
    let factor = Bps::from(BPS - reserve_factor.raw());
    Ray::from(factor.apply_to(env, rate_x_util.raw()))
}

pub fn compound_interest(env: &Env, rate: Ray, delta_ms: u64) -> Ray {
    if delta_ms == 0 {
        return Ray::ONE;
    }

    // dimensional: Ray<RatePerMs> * TimeMs -> Ray<1>; I256 guards extreme products.
    let x = Ray::from({
        let r = I256::from_i128(env, rate.raw());
        let d = I256::from_i128(env, delta_ms as i128);
        r.mul(&d)
            .to_i128()
            .unwrap_or_else(|| panic_with_error!(env, crate::errors::GenericError::MathOverflow))
    });

    // 8-term Taylor expansion of e^x. Remainder R8(x) ≤ x^9 / 9! → ≈ 0.14%
    // absolute error at x = 2. Per-chunk x is bounded by the accrual loop.
    let x_sq = x.mul(env, x);
    let x_cub = x_sq.mul(env, x);
    let x_pow4 = x_cub.mul(env, x);
    let x_pow5 = x_pow4.mul(env, x);
    let x_pow6 = x_pow5.mul(env, x);
    let x_pow7 = x_pow6.mul(env, x);
    let x_pow8 = x_pow7.mul(env, x);

    let term2 = x_sq.div_by_int(2);
    let term3 = x_cub.div_by_int(6);
    let term4 = x_pow4.div_by_int(24);
    let term5 = x_pow5.div_by_int(120);
    let term6 = x_pow6.div_by_int(720);
    let term7 = x_pow7.div_by_int(5_040);
    let term8 = x_pow8.div_by_int(40_320);

    let mut sum = Ray::ONE;
    for term in [x, term2, term3, term4, term5, term6, term7, term8] {
        sum.checked_add_assign(env, term);
    }
    sum
}

pub fn update_borrow_index(env: &Env, old_index: Ray, interest_factor: Ray) -> Ray {
    // dimensional: Ray<Index(asset, debt)> * Ray<1> -> Ray<Index(asset, debt)>.
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

    let protocol_fee = Ray::from(params.reserve_factor.apply_to(env, accrued_interest.raw()));
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
    let headroom = i128::MAX - supplied.raw();
    Ray::from(raw.min(headroom))
}

pub fn utilization(env: &Env, borrowed: Ray, supplied: Ray) -> Ray {
    if supplied == Ray::ZERO {
        return Ray::ZERO;
    }
    borrowed.div(env, supplied)
}

pub fn scaled_to_original(env: &Env, scaled: Ray, index: Ray) -> Ray {
    // dimensional: Ray<Share(asset, side)> * Ray<Index(asset, side)> -> Ray<Token(asset)>.
    scaled.mul(env, index)
}

/// Mint-path supply scaling: floor so minted shares never exceed cash in.
pub fn calculate_scaled_supply(env: &Env, amount: i128, decimals: u32, supply_index: Ray) -> Ray {
    Ray::from_asset(amount, decimals).div_floor(env, supply_index)
}

/// Burn-path supply scaling: ceil so cash out never exceeds burned value.
pub fn calculate_scaled_supply_ceil(
    env: &Env,
    amount: i128,
    decimals: u32,
    supply_index: Ray,
) -> Ray {
    Ray::from_asset(amount, decimals).div_ceil(env, supply_index)
}

/// Borrow-path debt scaling: ceil so recorded debt covers cash out.
pub fn calculate_scaled_borrow(env: &Env, amount: i128, decimals: u32, borrow_index: Ray) -> Ray {
    Ray::from_asset(amount, decimals).div_ceil(env, borrow_index)
}

/// Repay-path debt scaling: floor so debt burned never exceeds cash in.
pub fn calculate_scaled_borrow_floor(
    env: &Env,
    amount: i128,
    decimals: u32,
    borrow_index: Ray,
) -> Ray {
    Ray::from_asset(amount, decimals).div_floor(env, borrow_index)
}

/// Half-up supply unscale (full-close threshold / display).
pub fn unscale_supply(env: &Env, scaled: Ray, supply_index: Ray, decimals: u32) -> i128 {
    scaled_to_original(env, scaled, supply_index).to_asset(decimals)
}

/// Floor supply unscale (payouts, fee clamps, revenue claims).
pub fn unscale_supply_floor(env: &Env, scaled: Ray, supply_index: Ray, decimals: u32) -> i128 {
    scaled.mul_floor(env, supply_index).to_asset_floor(decimals)
}

/// Half-up borrow unscale.
pub fn unscale_borrow(env: &Env, scaled: Ray, borrow_index: Ray, decimals: u32) -> i128 {
    scaled_to_original(env, scaled, borrow_index).to_asset(decimals)
}

/// Ceil borrow unscale (debt owed / full-close amount).
pub fn unscale_borrow_ceil(env: &Env, scaled: Ray, borrow_index: Ray, decimals: u32) -> i128 {
    scaled.mul_ceil(env, borrow_index).to_asset_ceil(decimals)
}

/// Ceil borrow unscale kept in RAY (bad-debt write-down).
pub fn unscale_borrow_ceil_ray(env: &Env, scaled: Ray, borrow_index: Ray) -> Ray {
    scaled.mul_ceil(env, borrow_index)
}

/// Full-close when request ≥ half-up actual: burns all shares, pays floor gross.
/// Controller dust gates MUST mirror this rule or position map and pool diverge.
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

/// Full debt close when payment ≥ ceil owed; else floor-scale the repay.
/// Returns `(scaled_burned, overpayment)`.
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

/// Simulates index accrual without mutating pool storage.
/// Recomputes utilization and protocol revenue for each accrual chunk.
pub fn simulate_update_indexes(
    env: &Env,
    current_timestamp: u64,
    sync: &PoolSyncData,
) -> crate::types::MarketIndex {
    _simulate_update_indexes_impl(env, current_timestamp, sync)
}

#[cfg(not(feature = "certora"))]
fn _simulate_update_indexes_impl(
    env: &Env,
    current_timestamp: u64,
    sync: &PoolSyncData,
) -> crate::types::MarketIndex {
    simulate_update_indexes_body(env, current_timestamp, sync)
}

// Certora: monotone nondet summary (full Taylor expansion is intractable).
#[cfg(feature = "certora")]
cvlr_soroban_macros::apply_summary!(
    crate::spec::summaries::simulate_update_indexes_summary,
    pub(crate) fn _simulate_update_indexes_impl(
        env: &Env,
        current_timestamp: u64,
        sync: &PoolSyncData,
    ) -> crate::types::MarketIndex {
        simulate_update_indexes_body(env, current_timestamp, sync)
    }
);

pub(crate) fn simulate_update_indexes_body(
    env: &Env,
    current_timestamp: u64,
    sync: &PoolSyncData,
) -> crate::types::MarketIndex {
    let state = PoolState::from(&sync.state);
    let total_delta_ms = current_timestamp.saturating_sub(state.last_timestamp);

    if total_delta_ms == 0 {
        return crate::types::MarketIndex {
            supply_index: state.supply_index,
            borrow_index: state.borrow_index,
        };
    }

    let params = MarketParams::from(&sync.params);

    let mut supplied = state.supplied;
    let mut borrow_index = state.borrow_index;
    let mut supply_index = state.supply_index;

    let mut remaining = total_delta_ms;
    while remaining > 0 {
        let chunk = core::cmp::min(remaining, MAX_COMPOUND_DELTA_MS);

        let borrowed_original = scaled_to_original(env, state.borrowed, borrow_index);
        let supplied_original = scaled_to_original(env, supplied, supply_index);
        let util = utilization(env, borrowed_original, supplied_original);
        let borrow_rate = calculate_borrow_rate(env, util, &params);
        let interest_factor = compound_interest(env, borrow_rate, chunk);

        let new_borrow_index = update_borrow_index(env, borrow_index, interest_factor);

        let (supplier_rewards, protocol_fee) = calculate_supplier_rewards(
            env,
            &params,
            state.borrowed,
            new_borrow_index,
            borrow_index,
        );

        let old_supply_index = supply_index;
        supply_index = update_supply_index(env, supplied, old_supply_index, supplier_rewards);
        let supplier_shortfall = supply_index_reward_shortfall(
            env,
            supplied,
            old_supply_index,
            supply_index,
            supplier_rewards,
        );
        borrow_index = new_borrow_index;

        // Reserve fee plus virtual-offset shortfall mint scaled supply and feed
        // the next chunk's utilization exactly like mutating pool accrual.
        let protocol_reward = protocol_fee.checked_add(env, supplier_shortfall);
        if protocol_reward != Ray::ZERO {
            // Overflow-safe: a floored supply index can push the share count past
            // i128; `protocol_fee_shares` saturates and caps to remaining headroom.
            let fee_scaled = protocol_fee_shares(env, protocol_reward, supply_index, supplied);
            supplied = supplied.checked_add(env, fee_scaled);
        }

        remaining -= chunk;
    }

    crate::types::MarketIndex {
        supply_index,
        borrow_index,
    }
}

#[cfg(test)]
#[path = "../tests/rates.rs"]
mod tests;
