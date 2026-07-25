//! Kinked borrow-rate curve and the utilization it reads.
//!
//! Three segments joined at `mid_utilization` and `optimal_utilization`, capped
//! at `max_borrow_rate`. Rates are RAY per millisecond.

use soroban_sdk::Env;

use crate::constants::{BPS, MILLISECONDS_PER_YEAR};
use crate::math::fp::{Bps, Ray};
use crate::types::MarketParams;

/// Per-millisecond borrow rate from the three-segment kinked curve, capped at
/// `max_borrow_rate`.
pub fn calculate_borrow_rate(env: &Env, utilization: Ray, params: &MarketParams) -> Ray {
    // Clamp util >100% (degenerate over-cap state) to RAY so a near-RAY
    // optimal_utilization cannot overflow the slope3 term before the rate cap.
    let utilization = if utilization > Ray::ONE {
        Ray::ONE
    } else {
        utilization
    };
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

/// Per-millisecond supplier rate: the borrow rate scaled by utilization, less
/// the reserve factor.
///
/// Returns zero on an out-of-range `reserve_factor` rather than trapping —
/// `MarketParamsRaw::verify` rejects those at the boundary, so this is a
/// defensive floor, not the primary guard.
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
    let supplier_share = Bps::from(BPS - reserve_factor.raw());
    supplier_share.apply_to_ray(env, rate_x_util)
}

/// Borrowed over supplied, in RAY. Zero on an empty market.
pub fn utilization(env: &Env, borrowed: Ray, supplied: Ray) -> Ray {
    if supplied == Ray::ZERO {
        return Ray::ZERO;
    }
    borrowed.div(env, supplied)
}

#[cfg(test)]
#[path = "../../tests/rates/curve.rs"]
mod tests;
