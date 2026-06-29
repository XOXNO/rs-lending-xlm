use soroban_sdk::{panic_with_error, Env, I256};

use crate::constants::{BPS, MAX_BORROW_INDEX_RAY, MILLISECONDS_PER_YEAR, SUPPLY_INDEX_FLOOR_RAW};
use crate::math::fp::{Bps, Ray};
use crate::types::{MarketParams, PoolState, PoolSyncData};

/// Maximum compound-interest accrual chunk: one year in ms.
///
/// `compound_interest` is evaluated in chunks of at most this size.
pub const MAX_COMPOUND_DELTA_MS: u64 = MILLISECONDS_PER_YEAR;

/// Returns the per-millisecond borrow rate from the kinked utilization curve.
pub fn calculate_borrow_rate(env: &Env, utilization: Ray, params: &MarketParams) -> Ray {
    // dimensional: utilization is Ray<1>; model slopes are Ray<RatePerYear>.
    let annual_rate = if utilization < params.mid_utilization {
        let contribution = utilization
            .mul(env, params.slope1)
            .div(env, params.mid_utilization);
        params.base_borrow_rate + contribution
    } else if utilization < params.optimal_utilization {
        let excess = utilization - params.mid_utilization;
        let range = params.optimal_utilization - params.mid_utilization;
        let contribution = excess.mul(env, params.slope2).div(env, range);
        params.base_borrow_rate + params.slope1 + contribution
    } else {
        let base_rate = params.base_borrow_rate + params.slope1 + params.slope2;
        let excess = utilization - params.optimal_utilization;
        let range = Ray::ONE - params.optimal_utilization;
        let contribution = excess.mul(env, params.slope3).div(env, range);
        base_rate + contribution
    };

    let capped = if annual_rate > params.max_borrow_rate {
        params.max_borrow_rate
    } else {
        annual_rate
    };
    capped.div_by_int(MILLISECONDS_PER_YEAR as i128)
}

/// Returns supplier rate after reserve factor, in per-millisecond RAY units.
pub fn calculate_deposit_rate(
    env: &Env,
    utilization: Ray,
    borrow_rate: Ray,
    reserve_factor: Bps,
) -> Ray {
    if utilization == Ray::ZERO {
        return Ray::ZERO;
    }

    // Upstream rejects `reserve_factor >= BPS`; re-clamp to prevent
    // `BPS - reserve_factor` from going negative and inverting supplier rewards.
    if !(0..BPS).contains(&reserve_factor.raw()) {
        return Ray::ZERO;
    }

    let rate_x_util = utilization.mul(env, borrow_rate);
    let factor = Bps::from(BPS - reserve_factor.raw());
    Ray::from(factor.apply_to(env, rate_x_util.raw()))
}

/// Approximates `e^(rate_per_ms * delta_ms)` using an 8-term Taylor series.
pub fn compound_interest(env: &Env, rate: Ray, delta_ms: u64) -> Ray {
    if delta_ms == 0 {
        return Ray::ONE;
    }

    // dimensional: Ray<RatePerMs> * TimeMs -> Ray<1> compound factor exponent.
    // Intermediate promoted to I256 to guard against overflow on extreme products.
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

    Ray::ONE + x + term2 + term3 + term4 + term5 + term6 + term7 + term8
}

pub fn update_borrow_index(env: &Env, old_index: Ray, interest_factor: Ray) -> Ray {
    // dimensional: Ray<Index(asset, debt)> * Ray<1> -> Ray<Index(asset, debt)>.
    let new_index = old_index.mul(env, interest_factor);
    if new_index.raw() > MAX_BORROW_INDEX_RAY {
        return Ray::from(MAX_BORROW_INDEX_RAY);
    }
    new_index
}

/// Increases the supply index by distributing RAY-denominated rewards.
pub fn update_supply_index(env: &Env, supplied: Ray, old_index: Ray, rewards_increase: Ray) -> Ray {
    if supplied == Ray::ZERO || rewards_increase == Ray::ZERO {
        return old_index;
    }

    // dimensional: supplied * old_index and rewards_increase are Ray<Token(asset)>.
    let total_supplied_value = supplied.mul(env, old_index);
    // Guards the post-bad-debt path where `supplied * old_index` can round
    // to zero (supply_index at SUPPLY_INDEX_FLOOR with tiny scaled supply).
    if total_supplied_value == Ray::ZERO {
        return old_index;
    }
    // dimensional: rewards / total supplied -> Ray<1>; index scales by that factor.
    let rewards_ratio = rewards_increase.div(env, total_supplied_value);
    let factor = Ray::ONE + rewards_ratio;
    old_index.mul(env, factor)
}

/// Splits newly accrued borrow interest into supplier rewards and protocol fee.
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

    let accrued_interest = new_total_debt - old_total_debt;

    let protocol_fee = Ray::from(params.reserve_factor.apply_to(env, accrued_interest.raw()));
    let supplier_rewards = accrued_interest - protocol_fee;

    (supplier_rewards, protocol_fee)
}

/// Returns borrowed/supplied utilization, or zero when supplied is zero.
pub fn utilization(env: &Env, borrowed: Ray, supplied: Ray) -> Ray {
    if supplied == Ray::ZERO {
        return Ray::ZERO;
    }
    borrowed.div(env, supplied)
}

/// Converts scaled shares to underlying amount at `index`.
pub fn scaled_to_original(env: &Env, scaled: Ray, index: Ray) -> Ray {
    // dimensional: Ray<Share(asset, side)> * Ray<Index(asset, side)> -> Ray<Token(asset)>.
    scaled.mul(env, index)
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

// The read-path accrual loop runs an 8-term Taylor `compound_interest` per
// chunk. Under `certora`, the prover uses a monotone nondet index model; the
// production body is proved in `rates_rules::simulate_indexes_*`.
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

        supply_index = update_supply_index(env, supplied, supply_index, supplier_rewards);
        borrow_index = new_borrow_index;

        // Mirror `add_protocol_revenue`: the fee mints scaled supply, which
        // feeds the next chunk's utilization.
        if protocol_fee != Ray::ZERO
            && supply_index.raw() > SUPPLY_INDEX_FLOOR_RAW
            && supplied != Ray::ZERO
        {
            // dimensional: Ray<Token(asset)> / Ray<Index(asset, supply)> -> Ray<Share(asset, supply)>.
            let fee_scaled = protocol_fee.div(env, supply_index);
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
