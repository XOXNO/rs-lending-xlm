use soroban_sdk::{panic_with_error, Env, I256};

use crate::constants::{BPS, MILLISECONDS_PER_YEAR};
use crate::fp::{Bps, Ray};
use crate::types::MarketParams;

pub fn calculate_borrow_rate(env: &Env, utilization: Ray, params: &MarketParams) -> Ray {
    let mid = Ray::from_raw(params.mid_utilization_ray);
    let optimal = Ray::from_raw(params.optimal_utilization_ray);
    let base = Ray::from_raw(params.base_borrow_rate_ray);
    let s1 = Ray::from_raw(params.slope1_ray);
    let s2 = Ray::from_raw(params.slope2_ray);
    let s3 = Ray::from_raw(params.slope3_ray);
    let max_rate = Ray::from_raw(params.max_borrow_rate_ray);

    let annual_rate = if utilization < mid {
        // Region 1: below mid utilization
        let contribution = utilization.mul(env, s1).div(env, mid);
        base + contribution
    } else if utilization < optimal {
        // Region 2: between mid and optimal utilization
        let excess = utilization - mid;
        let range = optimal - mid;
        let contribution = excess.mul(env, s2).div(env, range);
        base + s1 + contribution
    } else {
        // Region 3: above optimal utilization
        let base_rate = base + s1 + s2;
        let excess = utilization - optimal;
        let range = Ray::ONE - optimal;
        let contribution = excess.mul(env, s3).div(env, range);
        base_rate + contribution
    };

    // Cap at max borrow rate, convert from annual to per-millisecond
    let capped = if annual_rate > max_rate {
        max_rate
    } else {
        annual_rate
    };
    capped.div_by_int(MILLISECONDS_PER_YEAR as i128)
}

pub fn calculate_deposit_rate(
    env: &Env,
    utilization: Ray,
    borrow_rate: Ray,
    reserve_factor_bps: i128,
) -> Ray {
    if utilization == Ray::ZERO {
        return Ray::ZERO;
    }

    // L-01 defense-in-depth: upstream validation rejects reserve_factor >=
    // BPS, but clamp here too so a mis-wired caller cannot produce negative
    // deposit rates (BPS - reserve_factor goes negative, then multiplying
    // rate_x_util flips sign and bogus supplier rewards accrue).
    if reserve_factor_bps < 0 || reserve_factor_bps >= BPS {
        return Ray::ZERO;
    }

    let rate_x_util = utilization.mul(env, borrow_rate);
    let factor = Bps::from_raw(BPS - reserve_factor_bps);
    Ray::from_raw(factor.apply_to(env, rate_x_util.raw()))
}

pub fn compound_interest(env: &Env, rate: Ray, delta_ms: u64) -> Ray {
    if delta_ms == 0 {
        return Ray::ONE;
    }

    // x = rate_per_ms * time_ms, both in RAY.
    // Uses I256 to prevent theoretical overflow for extreme rate * delta_ms products.
    let x = Ray::from_raw({
        let r = I256::from_i128(env, rate.raw());
        let d = I256::from_i128(env, delta_ms as i128);
        let result = r.mul(&d);
        result
            .to_i128()
            .unwrap_or_else(|| panic_with_error!(env, crate::errors::GenericError::MathOverflow))
    });

    // M-08: 8-term Taylor expansion of e^x. Error bound drops from ~1.66% at
    // x=2 (5 terms) to < 0.01% at x=2 (8 terms), keeping interest accurate
    // for markets idle up to 2 years at 100% borrow rate. Indexes still
    // accrue correctly on every user tx; 8 terms is defensive padding for
    // markets or keepers that fall behind.
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
    old_index.mul(env, interest_factor)
}

pub fn update_supply_index(
    env: &Env,
    supplied: Ray,
    old_index: Ray,
    rewards_increase: Ray,
) -> Ray {
    if supplied == Ray::ZERO || rewards_increase == Ray::ZERO {
        return old_index;
    }

    let total_supplied_value = supplied.mul(env, old_index);
    let rewards_ratio = rewards_increase.div(env, total_supplied_value);
    let factor = Ray::ONE + rewards_ratio;
    old_index.mul(env, factor)
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

    let accrued_interest = new_total_debt - old_total_debt;

    let protocol_fee =
        Ray::from_raw(Bps::from_raw(params.reserve_factor_bps).apply_to(env, accrued_interest.raw()));
    let supplier_rewards = accrued_interest - protocol_fee;

    (supplier_rewards, protocol_fee)
}

pub fn utilization(env: &Env, borrowed: Ray, supplied: Ray) -> Ray {
    if supplied == Ray::ZERO {
        return Ray::ZERO;
    }
    borrowed.div(env, supplied)
}

pub fn scaled_to_original(env: &Env, scaled: Ray, index: Ray) -> Ray {
    scaled.mul(env, index)
}

#[allow(clippy::too_many_arguments)]
pub fn simulate_update_indexes(
    env: &Env,
    current_timestamp: u64,
    last_timestamp: u64,
    borrowed: Ray,
    current_borrowed_index: Ray,
    supplied: Ray,
    current_supply_index: Ray,
    params: &MarketParams,
) -> crate::types::MarketIndex {
    let delta_ms = current_timestamp.saturating_sub(last_timestamp);

    if delta_ms > 0 {
        let borrowed_original = scaled_to_original(env, borrowed, current_borrowed_index);
        let supplied_original = scaled_to_original(env, supplied, current_supply_index);
        let util = utilization(env, borrowed_original, supplied_original);
        let borrow_rate = calculate_borrow_rate(env, util, params);
        let interest_factor = compound_interest(env, borrow_rate, delta_ms);

        let new_borrow_index = update_borrow_index(env, current_borrowed_index, interest_factor);

        let (supplier_rewards, _) = calculate_supplier_rewards(
            env,
            params,
            borrowed,
            new_borrow_index,
            current_borrowed_index,
        );

        let new_supply_index =
            update_supply_index(env, supplied, current_supply_index, supplier_rewards);

        crate::types::MarketIndex {
            supply_index_ray: new_supply_index.raw(),
            borrow_index_ray: new_borrow_index.raw(),
        }
    } else {
        crate::types::MarketIndex {
            supply_index_ray: current_supply_index.raw(),
            borrow_index_ray: current_borrowed_index.raw(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::RAY;
    use crate::fp_core::div_by_int_half_up;
    use soroban_sdk::Env;

    fn make_test_params() -> MarketParams {
        MarketParams {
            base_borrow_rate_ray: RAY / 100,         // 1%
            slope1_ray: RAY * 4 / 100,               // 4%
            slope2_ray: RAY * 10 / 100,              // 10%
            slope3_ray: RAY * 300 / 100,             // 300%
            mid_utilization_ray: RAY * 50 / 100,     // 50%
            optimal_utilization_ray: RAY * 80 / 100, // 80%
            max_borrow_rate_ray: RAY,                // 100%
            reserve_factor_bps: 1000,                // 10%
            asset_id: soroban_sdk::Address::from_str(
                &Env::default(),
                "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
            ),
            asset_decimals: 7,
        }
    }

    #[test]
    fn test_borrow_rate_region1() {
        let env = Env::default();
        let params = make_test_params();

        let rate = calculate_borrow_rate(&env, Ray::ZERO, &params);
        let expected = div_by_int_half_up(RAY / 100, MILLISECONDS_PER_YEAR as i128);
        assert_eq!(rate.raw(), expected);

        let util_25 = Ray::from_raw(RAY * 25 / 100);
        let rate = calculate_borrow_rate(&env, util_25, &params);
        let expected_annual = RAY * 3 / 100;
        let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
        assert!((rate.raw() - expected_per_ms).abs() <= 1);
    }

    #[test]
    fn test_borrow_rate_region2() {
        let env = Env::default();
        let params = make_test_params();

        let util_50 = Ray::from_raw(RAY * 50 / 100);
        let rate = calculate_borrow_rate(&env, util_50, &params);
        let expected_annual = RAY * 5 / 100;
        let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
        assert!((rate.raw() - expected_per_ms).abs() <= 1);

        let util_65 = Ray::from_raw(RAY * 65 / 100);
        let rate = calculate_borrow_rate(&env, util_65, &params);
        let expected_annual = RAY * 10 / 100;
        let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
        assert!((rate.raw() - expected_per_ms).abs() <= 1);
    }

    #[test]
    fn test_borrow_rate_region3() {
        let env = Env::default();
        let params = make_test_params();

        let util_80 = Ray::from_raw(RAY * 80 / 100);
        let rate = calculate_borrow_rate(&env, util_80, &params);
        let expected_annual = RAY * 15 / 100;
        let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
        assert!((rate.raw() - expected_per_ms).abs() <= 1);

        let util_90 = Ray::from_raw(RAY * 90 / 100);
        let rate = calculate_borrow_rate(&env, util_90, &params);
        let expected_annual = RAY;
        let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
        assert!((rate.raw() - expected_per_ms).abs() <= 1);
    }

    #[test]
    fn test_borrow_rate_capped() {
        let env = Env::default();
        let params = make_test_params();

        let rate = calculate_borrow_rate(&env, Ray::ONE, &params);
        let expected =
            div_by_int_half_up(params.max_borrow_rate_ray, MILLISECONDS_PER_YEAR as i128);
        assert!((rate.raw() - expected).abs() <= 1);
    }

    #[test]
    fn test_compound_interest_zero_delta() {
        let env = Env::default();
        let result = compound_interest(&env, Ray::from_raw(RAY / 10), 0);
        assert_eq!(result, Ray::ONE);
    }

    #[test]
    fn test_compound_interest_accuracy() {
        let env = Env::default();

        let annual_rate = Ray::from_raw(RAY / 10);
        let rate_per_ms = annual_rate.div_by_int(MILLISECONDS_PER_YEAR as i128);
        let factor = compound_interest(&env, rate_per_ms, MILLISECONDS_PER_YEAR);

        let expected_e_010 = 1_105_170_918_075_647_624_811_707_826_i128;

        let diff = (factor.raw() - expected_e_010).abs();
        let tolerance = expected_e_010 / 1_000_000;
        assert!(
            diff < tolerance,
            "Compound interest accuracy: factor={}, expected={}, diff={}, tolerance={}",
            factor.raw(),
            expected_e_010,
            diff,
            tolerance
        );
    }

    #[test]
    fn test_update_borrow_index() {
        let env = Env::default();
        let old_index = Ray::ONE;
        let factor = Ray::from_raw(RAY + RAY * 5 / 100);
        let new_index = update_borrow_index(&env, old_index, factor);
        let expected = RAY * 105 / 100;
        assert!((new_index.raw() - expected).abs() <= 1);
    }

    #[test]
    fn test_update_supply_index() {
        let env = Env::default();
        let supplied = Ray::from_raw(100 * RAY);
        let old_index = Ray::ONE;
        let rewards = Ray::from_raw(5 * RAY);
        let new_index = update_supply_index(&env, supplied, old_index, rewards);
        let expected = RAY * 105 / 100;
        assert!((new_index.raw() - expected).abs() <= 1);
    }

    #[test]
    fn test_update_supply_index_zero_supplied() {
        let env = Env::default();
        let result = update_supply_index(&env, Ray::ZERO, Ray::ONE, Ray::from_raw(5 * RAY));
        assert_eq!(result, Ray::ONE);
    }

    #[test]
    fn test_utilization_basic() {
        let env = Env::default();
        let util = utilization(&env, Ray::from_raw(50 * RAY), Ray::from_raw(100 * RAY));
        let expected = RAY / 2;
        assert!((util.raw() - expected).abs() <= 1);
    }

    #[test]
    fn test_utilization_zero_supplied() {
        let env = Env::default();
        assert_eq!(utilization(&env, Ray::from_raw(50 * RAY), Ray::ZERO), Ray::ZERO);
    }

    #[test]
    fn test_scaled_to_original() {
        let env = Env::default();
        let scaled = Ray::from_raw(100 * RAY);
        let index = Ray::from_raw(RAY * 105 / 100);
        let result = scaled_to_original(&env, scaled, index);
        let expected = 105 * RAY;
        assert!((result.raw() - expected).abs() <= 1);
    }

    #[test]
    fn test_calculate_supplier_rewards() {
        let env = Env::default();
        let params = make_test_params();

        let borrowed = Ray::from_raw(100 * RAY);
        let old_index = Ray::ONE;
        let new_index = Ray::from_raw(RAY + RAY / 100);

        let (rewards, fee) =
            calculate_supplier_rewards(&env, &params, borrowed, new_index, old_index);

        let expected_fee = RAY / 10;
        let expected_rewards = RAY * 9 / 10;

        assert!(
            (fee.raw() - expected_fee).abs() <= 1,
            "fee={}, expected={}",
            fee.raw(),
            expected_fee
        );
        assert!(
            (rewards.raw() - expected_rewards).abs() <= 1,
            "rewards={}, expected={}",
            rewards.raw(),
            expected_rewards
        );
    }

    #[test]
    fn test_deposit_rate() {
        let env = Env::default();
        let util_80 = Ray::from_raw(RAY * 80 / 100);
        let borrow_rate = Ray::from_raw(RAY * 5 / 100);
        let reserve_factor = 1000;

        let rate = calculate_deposit_rate(&env, util_80, borrow_rate, reserve_factor);

        let expected = RAY * 36 / 1000;
        assert!(
            (rate.raw() - expected).abs() <= 1,
            "rate={}, expected={}",
            rate.raw(),
            expected
        );
    }

    #[test]
    fn test_deposit_rate_zero_util() {
        let env = Env::default();
        assert_eq!(
            calculate_deposit_rate(&env, Ray::ZERO, Ray::from_raw(RAY / 10), 1000),
            Ray::ZERO
        );
    }
}
