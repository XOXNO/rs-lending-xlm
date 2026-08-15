//! Computes borrow and deposit interest rates and pool utilization from a
//! three-segment piecewise-linear rate curve.

use soroban_sdk::Env;

use crate::constants::{BPS, MILLISECONDS_PER_YEAR};
use crate::math::fp::{Bps, Ray};
use crate::types::MarketParams;

/// Computes the annual borrow APR (RAY) for `utilization` under the
/// three-segment piecewise-linear curve defined by `params`.
///
/// Clamps `utilization` to at most `Ray::ONE`. Below `params.mid_utilization`
/// the annual rate ramps from `params.base_borrow_rate` by `params.slope1`;
/// between `mid_utilization` and `params.optimal_utilization` it ramps
/// further by `params.slope2`; above `optimal_utilization` it ramps by
/// `params.slope3`. Caps the result at `params.max_borrow_rate`.
///
/// This is the value pool view getters return. Accrual still converts it to
/// a per-millisecond rate via [`calculate_borrow_rate`].
pub fn calculate_annual_borrow_rate(env: &Env, utilization: Ray, params: &MarketParams) -> Ray {
    let utilization = if utilization > Ray::ONE {
        Ray::ONE
    } else {
        utilization
    };

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

    if annual_rate > params.max_borrow_rate {
        params.max_borrow_rate
    } else {
        annual_rate
    }
}

/// Computes the per-millisecond borrow rate for `utilization`.
///
/// Divides [`calculate_annual_borrow_rate`] by [`MILLISECONDS_PER_YEAR`] so
/// [`compound_interest`] can scale by elapsed milliseconds.
pub fn calculate_borrow_rate(env: &Env, utilization: Ray, params: &MarketParams) -> Ray {
    calculate_annual_borrow_rate(env, utilization, params).div_by_int(MILLISECONDS_PER_YEAR as i128)
}

/// Computes the deposit rate suppliers earn from `borrow_rate` at the given
/// `utilization`, after deducting `reserve_factor`.
///
/// The result keeps the same time unit as `borrow_rate`: annual RAY when the
/// input is an annual APR, per-millisecond RAY when the input is the accrual
/// rate.
///
/// Returns `Ray::ZERO` if `utilization` is zero or if `reserve_factor` is not
/// in the range `0..BPS`.
pub fn calculate_deposit_rate(
    env: &Env,
    utilization: Ray,
    borrow_rate: Ray,
    reserve_factor: Bps,
) -> Ray {
    if utilization == Ray::ZERO {
        return Ray::ZERO;
    }

    if !(0..BPS).contains(&reserve_factor.raw()) {
        return Ray::ZERO;
    }

    let rate_x_util = utilization.mul(env, borrow_rate);
    let supplier_share = Bps::from(BPS - reserve_factor.raw());
    supplier_share.apply_to_ray(env, rate_x_util)
}

/// Computes the pool utilization ratio `borrowed / supplied`. Returns
/// `Ray::ZERO` if `supplied` is zero.
pub fn utilization(env: &Env, borrowed: Ray, supplied: Ray) -> Ray {
    if supplied == Ray::ZERO {
        return Ray::ZERO;
    }
    borrowed.div(env, supplied)
}

#[cfg(test)]
#[path = "../../tests/rates/curve.rs"]
mod tests;
