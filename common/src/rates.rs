use soroban_sdk::{assert_with_error, panic_with_error, Env, I256};

use crate::constants::{BPS, MAX_BORROW_INDEX_RAY, MILLISECONDS_PER_YEAR};
use crate::errors::GenericError;
use crate::math::fp::{Bps, Ray};
use crate::types::{MarketParams, PoolState, PoolSyncData};

/// Returns the per-millisecond borrow rate from the kinked utilization curve.
pub fn calculate_borrow_rate(env: &Env, utilization: Ray, params: &MarketParams) -> Ray {
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

    // Defense-in-depth: upstream rejects `reserve_factor >= BPS`; re-clamp so a mis-wired
    // caller cannot drive `BPS - reserve_factor` negative and invert the supplier-reward sign.
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

    // x = rate_per_ms * time_ms, both in RAY. Intermediate promoted to I256
    // to guard against overflow on extreme rate * delta_ms products.
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
    let new_index = old_index.mul(env, interest_factor);
    assert_with_error!(
        env,
        new_index.raw() <= MAX_BORROW_INDEX_RAY,
        GenericError::MathOverflow
    );
    new_index
}

/// Increases the supply index by distributing RAY-denominated rewards.
pub fn update_supply_index(env: &Env, supplied: Ray, old_index: Ray, rewards_increase: Ray) -> Ray {
    if supplied == Ray::ZERO || rewards_increase == Ray::ZERO {
        return old_index;
    }

    let total_supplied_value = supplied.mul(env, old_index);
    // Guards the post-bad-debt path where `supplied * old_index` can round
    // to zero (supply_index at SUPPLY_INDEX_FLOOR with tiny scaled supply).
    if total_supplied_value == Ray::ZERO {
        return old_index;
    }
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
    scaled.mul(env, index)
}

/// Simulates index accrual without mutating pool storage.
pub fn simulate_update_indexes(
    env: &Env,
    current_timestamp: u64,
    sync: &PoolSyncData,
) -> crate::types::MarketIndex {
    let state = PoolState::from(&sync.state);
    let delta_ms = current_timestamp.saturating_sub(state.last_timestamp);

    if delta_ms == 0 {
        return crate::types::MarketIndex {
            supply_index: state.supply_index,
            borrow_index: state.borrow_index,
        };
    }

    let params = MarketParams::from(&sync.params);

    let borrowed_original = scaled_to_original(env, state.borrowed, state.borrow_index);
    let supplied_original = scaled_to_original(env, state.supplied, state.supply_index);
    let util = utilization(env, borrowed_original, supplied_original);
    let borrow_rate = calculate_borrow_rate(env, util, &params);
    let interest_factor = compound_interest(env, borrow_rate, delta_ms);

    let new_borrow_index = update_borrow_index(env, state.borrow_index, interest_factor);

    let (supplier_rewards, _) = calculate_supplier_rewards(
        env,
        &params,
        state.borrowed,
        new_borrow_index,
        state.borrow_index,
    );

    let new_supply_index =
        update_supply_index(env, state.supplied, state.supply_index, supplier_rewards);

    crate::types::MarketIndex {
        supply_index: new_supply_index,
        borrow_index: new_borrow_index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::RAY;
    use crate::math::fp_core::div_by_int_half_up;
    use soroban_sdk::Env;

    fn make_test_params() -> MarketParams {
        MarketParams {
            base_borrow_rate: Ray::from(RAY / 100),         // 1%
            slope1: Ray::from(RAY * 4 / 100),               // 4%
            slope2: Ray::from(RAY * 10 / 100),              // 10%
            slope3: Ray::from(RAY * 300 / 100),             // 300%
            mid_utilization: Ray::from(RAY * 50 / 100),     // 50%
            optimal_utilization: Ray::from(RAY * 80 / 100), // 80%
            max_utilization: Ray::from(RAY * 95 / 100),     // 95%
            max_borrow_rate: Ray::from(RAY),                // 100%
            reserve_factor: Bps::from(1000),                // 10%
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

        let util_25 = Ray::from(RAY * 25 / 100);
        let rate = calculate_borrow_rate(&env, util_25, &params);
        let expected_annual = RAY * 3 / 100;
        let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
        assert!((rate.raw() - expected_per_ms).abs() <= 1);
    }

    #[test]
    fn test_borrow_rate_region2() {
        let env = Env::default();
        let params = make_test_params();

        let util_50 = Ray::from(RAY * 50 / 100);
        let rate = calculate_borrow_rate(&env, util_50, &params);
        let expected_annual = RAY * 5 / 100;
        let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
        assert!((rate.raw() - expected_per_ms).abs() <= 1);

        let util_65 = Ray::from(RAY * 65 / 100);
        let rate = calculate_borrow_rate(&env, util_65, &params);
        let expected_annual = RAY * 10 / 100;
        let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
        assert!((rate.raw() - expected_per_ms).abs() <= 1);
    }

    #[test]
    fn test_borrow_rate_region3() {
        let env = Env::default();
        let params = make_test_params();

        let util_80 = Ray::from(RAY * 80 / 100);
        let rate = calculate_borrow_rate(&env, util_80, &params);
        let expected_annual = RAY * 15 / 100;
        let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
        assert!((rate.raw() - expected_per_ms).abs() <= 1);

        let util_90 = Ray::from(RAY * 90 / 100);
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
            div_by_int_half_up(params.max_borrow_rate.raw(), MILLISECONDS_PER_YEAR as i128);
        assert!((rate.raw() - expected).abs() <= 1);
    }

    #[test]
    fn test_compound_interest_zero_delta() {
        let env = Env::default();
        let result = compound_interest(&env, Ray::from(RAY / 10), 0);
        assert_eq!(result, Ray::ONE);
    }

    #[test]
    fn test_compound_interest_accuracy() {
        let env = Env::default();

        let annual_rate = Ray::from(RAY / 10);
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
        let factor = Ray::from(RAY + RAY * 5 / 100);
        let new_index = update_borrow_index(&env, old_index, factor);
        let expected = RAY * 105 / 100;
        assert!((new_index.raw() - expected).abs() <= 1);
    }

    #[test]
    fn test_update_supply_index() {
        let env = Env::default();
        let supplied = Ray::from(100 * RAY);
        let old_index = Ray::ONE;
        let rewards = Ray::from(5 * RAY);
        let new_index = update_supply_index(&env, supplied, old_index, rewards);
        let expected = RAY * 105 / 100;
        assert!((new_index.raw() - expected).abs() <= 1);
    }

    #[test]
    fn test_update_supply_index_zero_supplied() {
        let env = Env::default();
        let result = update_supply_index(&env, Ray::ZERO, Ray::ONE, Ray::from(5 * RAY));
        assert_eq!(result, Ray::ONE);
    }

    #[test]
    fn test_utilization_basic() {
        let env = Env::default();
        let util = utilization(&env, Ray::from(50 * RAY), Ray::from(100 * RAY));
        let expected = RAY / 2;
        assert!((util.raw() - expected).abs() <= 1);
    }

    #[test]
    fn test_utilization_zero_supplied() {
        let env = Env::default();
        assert_eq!(utilization(&env, Ray::from(50 * RAY), Ray::ZERO), Ray::ZERO);
    }

    #[test]
    fn test_scaled_to_original() {
        let env = Env::default();
        let scaled = Ray::from(100 * RAY);
        let index = Ray::from(RAY * 105 / 100);
        let result = scaled_to_original(&env, scaled, index);
        let expected = 105 * RAY;
        assert!((result.raw() - expected).abs() <= 1);
    }

    #[test]
    fn test_calculate_supplier_rewards() {
        let env = Env::default();
        let params = make_test_params();

        let borrowed = Ray::from(100 * RAY);
        let old_index = Ray::ONE;
        let new_index = Ray::from(RAY + RAY / 100);

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
        let util_80 = Ray::from(RAY * 80 / 100);
        let borrow_rate = Ray::from(RAY * 5 / 100);
        let reserve_factor = Bps::from(1000);

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
            calculate_deposit_rate(&env, Ray::ZERO, Ray::from(RAY / 10), Bps::from(1000)),
            Ray::ZERO
        );
    }

    // `update_borrow_index` boundary: `new_index > MAX` must panic, `==` must
    // not. Differentiates `>` from `==`/`>=` on line 99.

    #[test]
    fn test_update_borrow_index_at_max_does_not_panic() {
        let env = Env::default();
        let old_index = Ray::from(MAX_BORROW_INDEX_RAY);
        let new_index = update_borrow_index(&env, old_index, Ray::ONE);
        assert_eq!(new_index.raw(), MAX_BORROW_INDEX_RAY);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #33)")]
    fn test_update_borrow_index_above_max_panics() {
        let env = Env::default();
        let old_index = Ray::from(MAX_BORROW_INDEX_RAY);
        // factor = 1 + 1 ulp → product strictly exceeds MAX.
        let factor = Ray::from(RAY + 1);
        let _ = update_borrow_index(&env, old_index, factor);
    }

    // `simulate_update_indexes` early-return guard `if delta_ms == 0`: with a
    // nonzero delta and live borrows the original accrues (indexes grow).
    // Mutating `==`→`!=` would return the input indexes unchanged; asserting the
    // borrow index strictly increased kills that mutant.
    #[test]
    fn test_simulate_update_indexes_nonzero_delta_accrues() {
        use crate::types::{MarketParamsRaw, PoolStateRaw, PoolSyncData};

        let env = Env::default();
        let params = MarketParamsRaw {
            max_borrow_rate_ray: RAY,
            base_borrow_rate_ray: RAY / 100,
            slope1_ray: RAY * 4 / 100,
            slope2_ray: RAY * 10 / 100,
            slope3_ray: RAY * 300 / 100,
            mid_utilization_ray: RAY * 50 / 100,
            optimal_utilization_ray: RAY * 80 / 100,
            max_utilization_ray: RAY * 95 / 100,
            reserve_factor_bps: 1_000,
            asset_id: soroban_sdk::Address::from_str(
                &env,
                "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
            ),
            asset_decimals: 7,
        };
        let state = PoolStateRaw {
            supplied_ray: 100 * RAY,
            borrowed_ray: 60 * RAY,
            revenue_ray: 0,
            borrow_index_ray: RAY,
            supply_index_ray: RAY,
            last_timestamp: 0,
        };
        let sync = PoolSyncData { params, state };

        // delta_ms > 0 → original accrues interest.
        let one_year = MILLISECONDS_PER_YEAR;
        let indexes = simulate_update_indexes(&env, one_year, &sync);
        assert!(
            indexes.borrow_index.raw() > RAY,
            "borrow index must grow over a nonzero delta; got {}",
            indexes.borrow_index.raw()
        );
        assert!(
            indexes.supply_index.raw() > RAY,
            "supply index must grow over a nonzero delta; got {}",
            indexes.supply_index.raw()
        );
    }

    // Pins compound_interest against e^0.5 with tolerance tight enough to
    // detect a sign flip on any Taylor term (term2..term8). Truncation
    // bound at x = 0.5 is x^9/9! ≈ 5.4e-9 → 5.4e18 in Ray units.
    #[test]
    fn test_compound_interest_high_x_pins_all_taylor_terms() {
        let env = Env::default();
        // rate * delta = x = 0.5 Ray. Set rate = 0.5 RAY/ms, delta = 1.
        let rate = Ray::from(RAY / 2);
        let result = compound_interest(&env, rate, 1);

        // e^0.5 = 1.6487212707001281468486507878...
        let expected = 1_648_721_270_700_128_146_848_650_787_i128;

        // Tolerance must be >> Taylor truncation (5.4e18) but << any single
        // term's magnitude. Smallest relevant term is term8 ≈ 1.9e20.
        let tolerance = 1e19 as i128;
        let diff = (result.raw() - expected).abs();
        assert!(
            diff <= tolerance,
            "compound_interest(0.5) drift {} exceeds tolerance {}; got {}, expected {}",
            diff,
            tolerance,
            result.raw(),
            expected
        );
    }
}
