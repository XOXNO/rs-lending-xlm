//! Computes borrow and deposit interest rates and pool utilization from a
//! three-segment piecewise-linear rate curve.

use soroban_sdk::{panic_with_error, Env};

use crate::constants::{BPS, MILLISECONDS_PER_YEAR};
use crate::errors::GenericError;
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
/// Each segment's contribution is one half-up multiply-divide, so at the top
/// of a segment it equals the slope exactly and the uncapped rate never
/// decreases as utilization rises.
///
/// Pool rate views return this annual rate. Accrual uses the per-millisecond
/// rate from [`calculate_borrow_rate`].
pub fn calculate_annual_borrow_rate(env: &Env, utilization: Ray, params: &MarketParams) -> Ray {
    let utilization = if utilization > Ray::ONE {
        Ray::ONE
    } else {
        utilization
    };

    let annual_rate = if utilization < params.mid_utilization {
        let contribution =
            slope_contribution(env, utilization, params.slope1, params.mid_utilization);
        params.base_borrow_rate.checked_add(env, contribution)
    } else if utilization < params.optimal_utilization {
        let excess = utilization.checked_sub(env, params.mid_utilization);
        let range = params
            .optimal_utilization
            .checked_sub(env, params.mid_utilization);
        let contribution = slope_contribution(env, excess, params.slope2, range);
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
        let contribution = slope_contribution(env, excess, params.slope3, range);
        base_rate.checked_add(env, contribution)
    };

    if annual_rate > params.max_borrow_rate {
        params.max_borrow_rate
    } else {
        annual_rate
    }
}

/// Computes the share of `slope` earned `excess` of the way through a segment
/// `range` wide: `slope * excess / range`, rounded half up once.
///
/// Rounding `excess * slope` before dividing by `range` would scale that
/// first rounding error by `RAY / range`, up to half of `RAY` for a segment
/// one unit wide.
fn slope_contribution(env: &Env, excess: Ray, slope: Ray, range: Ray) -> Ray {
    slope.mul_ray_ratio(env, excess, range)
}

/// Computes the per-millisecond borrow rate (RAY) for `utilization`.
///
/// Divides [`calculate_annual_borrow_rate`] by [`MILLISECONDS_PER_YEAR`],
/// rounding half up.
pub fn calculate_borrow_rate(env: &Env, utilization: Ray, params: &MarketParams) -> Ray {
    calculate_annual_borrow_rate(env, utilization, params)
        .div_by_int(env, MILLISECONDS_PER_YEAR as i128)
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

/// Computes the pool utilization ratio `borrowed / supplied`, rounded half
/// up and capped at `Ray::ONE`. The result is always in `[0, Ray::ONE]`.
///
/// Returns `Ray::ZERO` if `supplied` is zero, and `Ray::ONE` if `borrowed` is
/// at least `supplied`. Only a `borrowed` below `supplied` is divided, so the
/// quotient cannot overflow.
///
/// Panics with `GenericError::MathOverflow` if `borrowed` or `supplied` is
/// negative.
pub fn utilization(env: &Env, borrowed: Ray, supplied: Ray) -> Ray {
    if supplied == Ray::ZERO {
        return Ray::ZERO;
    }
    if borrowed < Ray::ZERO || supplied < Ray::ZERO {
        panic_with_error!(env, GenericError::MathOverflow);
    }
    if borrowed >= supplied {
        return Ray::ONE;
    }
    borrowed.div(env, supplied)
}

#[cfg(test)]
#[path = "../../tests/rates/curve.rs"]
mod tests;
