use super::*;
use crate::constants::{MILLISECONDS_PER_YEAR, RAY, SUPPLY_INDEX_FLOOR_RAW};
use crate::rates::test_support::*;
use crate::types::PoolStateRaw;
use soroban_sdk::Env;

#[test]
fn test_simulate_update_indexes_nonzero_delta_accrues() {
    let env = Env::default();
    let sync = sample_sync(
        &env,
        PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 60 * RAY,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 40_000_000,
        },
    );

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

#[test]
fn test_simulate_update_indexes_multi_year_exceeds_single_shot() {
    let env = Env::default();
    let sync = sample_sync(
        &env,
        PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 90 * RAY,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 10_000_000,
        },
    );
    let p = MarketParams::from(&sync.params);
    let s = PoolState::from(&sync.state);

    let two_years = 2 * MILLISECONDS_PER_YEAR;
    let chunked = simulate_update_indexes(&env, two_years, &sync);

    let util = utilization(
        &env,
        scaled_to_original(&env, s.borrowed, s.borrow_index),
        scaled_to_original(&env, s.supplied, s.supply_index),
    );
    let rate = calculate_borrow_rate(&env, util, &p);
    let single_shot = update_borrow_index(
        &env,
        s.borrow_index,
        compound_interest(&env, rate, two_years),
    );

    assert!(
        chunked.borrow_index.raw() > single_shot.raw(),
        "chunked 2y accrual {} must exceed single-shot {}",
        chunked.borrow_index.raw(),
        single_shot.raw()
    );

    let one_year = simulate_update_indexes(&env, MILLISECONDS_PER_YEAR, &sync);
    assert!(chunked.borrow_index.raw() > one_year.borrow_index.raw());
}

#[test]
fn test_simulate_update_indexes_zero_delta_is_noop() {
    let env = Env::default();
    let sync = sample_sync(
        &env,
        PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 60 * RAY,
            revenue: 0,
            borrow_index: 2 * RAY,
            supply_index: 3 * RAY,
            last_timestamp: 1_000,
            cash: 40_000_000,
        },
    );

    let indexes = simulate_update_indexes(&env, 1_000, &sync);
    assert_eq!(indexes.borrow_index, Ray::from(2 * RAY));
    assert_eq!(indexes.supply_index, Ray::from(3 * RAY));
}

#[test]
fn test_simulate_guard_reinvests_fee_when_healthy() {
    let env = Env::default();
    let raw_params = make_test_params_raw(&env);
    let sync = sample_sync(
        &env,
        PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 60 * RAY,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 40_000_000,
        },
    );
    let params = MarketParams::from(&raw_params);

    let two_years = 2 * MILLISECONDS_PER_YEAR;
    let actual = simulate_update_indexes(&env, two_years, &sync);

    let (expected_borrow_index, expected_supply_index) = oracle_accrual(
        &env,
        &params,
        Ray::from(60 * RAY),
        Ray::from(100 * RAY),
        Ray::ONE,
        Ray::ONE,
        &[MAX_COMPOUND_DELTA_MS, MAX_COMPOUND_DELTA_MS],
    );

    assert_eq!(actual.borrow_index.raw(), expected_borrow_index.raw());
    assert_eq!(actual.supply_index.raw(), expected_supply_index.raw());
}

#[test]
fn test_simulate_matches_mirror_when_supplied_zero() {
    let env = Env::default();
    let raw_params = make_test_params_raw(&env);
    let sync = sample_sync(
        &env,
        PoolStateRaw {
            supplied: 0,
            borrowed: 60 * RAY,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 40_000_000,
        },
    );
    let params = MarketParams::from(&raw_params);

    let two_years = 2 * MILLISECONDS_PER_YEAR;
    let actual = simulate_update_indexes(&env, two_years, &sync);

    let (expected_borrow_index, expected_supply_index) = oracle_accrual(
        &env,
        &params,
        Ray::from(60 * RAY),
        Ray::ZERO,
        Ray::ONE,
        Ray::ONE,
        &[MAX_COMPOUND_DELTA_MS, MAX_COMPOUND_DELTA_MS],
    );

    assert_eq!(actual.borrow_index.raw(), expected_borrow_index.raw());
    assert_eq!(actual.supply_index.raw(), expected_supply_index.raw());
}

#[test]
fn test_simulate_matches_mirror_at_supply_index_floor() {
    let env = Env::default();

    let mut raw_params = make_test_params_raw(&env);
    raw_params.reserve_factor = 10_000;
    let raw_state = PoolStateRaw {
        supplied: 100 * RAY,
        borrowed: 60 * RAY,
        revenue: 0,
        borrow_index: RAY,
        supply_index: SUPPLY_INDEX_FLOOR_RAW,
        last_timestamp: 0,
        cash: 40_000_000,
    };
    let params = MarketParams::from(&raw_params);
    let sync = PoolSyncData {
        params: raw_params,
        state: raw_state,
    };

    let two_years = 2 * MILLISECONDS_PER_YEAR;
    let actual = simulate_update_indexes(&env, two_years, &sync);

    let (expected_borrow_index, expected_supply_index) = oracle_accrual(
        &env,
        &params,
        Ray::from(60 * RAY),
        Ray::from(100 * RAY),
        Ray::ONE,
        Ray::from(SUPPLY_INDEX_FLOOR_RAW),
        &[MAX_COMPOUND_DELTA_MS, MAX_COMPOUND_DELTA_MS],
    );

    assert_eq!(actual.borrow_index.raw(), expected_borrow_index.raw());
    assert_eq!(actual.supply_index.raw(), expected_supply_index.raw());
}

#[test]
fn test_split_accrual_never_reduces_borrow_index() {
    let env = Env::default();
    let params = make_test_params_raw(&env);
    let mk = |borrow_index: i128, supply_index: i128, last_timestamp: u64| PoolSyncData {
        params: params.clone(),
        state: PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 80 * RAY,
            revenue: 0,
            borrow_index,
            supply_index,
            last_timestamp,
            cash: 20_000_000,
        },
    };

    let total = MAX_COMPOUND_DELTA_MS / 2;
    let single = simulate_update_indexes(&env, total, &mk(RAY, RAY, 0));

    let split_at = total * 3 / 7;
    let step1 = simulate_update_indexes(&env, split_at, &mk(RAY, RAY, 0));
    let split = simulate_update_indexes(
        &env,
        total,
        &mk(step1.borrow_index.raw(), step1.supply_index.raw(), split_at),
    );

    assert!(
        split.borrow_index.raw() >= single.borrow_index.raw(),
        "split must not lower borrow index: split={} single={}",
        split.borrow_index.raw(),
        single.borrow_index.raw()
    );
    assert!(
        split.supply_index.raw() >= single.supply_index.raw(),
        "split must not lower supply index: split={} single={}",
        split.supply_index.raw(),
        single.supply_index.raw()
    );

    assert!(
        split.borrow_index.raw() <= single.borrow_index.raw() * 2,
        "split ran away vs single: split={} single={}",
        split.borrow_index.raw(),
        single.borrow_index.raw()
    );
}

// --- accrue_step boundaries (GH-04) ---------------------------------------

mod accrue_step_boundaries {
    use super::*;
    use crate::math::fp::Ray;
    use crate::rates::{calculate_borrow_rate, utilization};

    #[test]
    fn zero_delta_is_identity_with_no_revenue() {
        let env = Env::default();
        let params = make_test_params(&env);
        let step = accrue_step(
            &env,
            &params,
            Ray::from(60 * RAY),
            Ray::from(100 * RAY),
            Ray::ONE,
            Ray::ONE,
            0,
        );
        assert_eq!(step.borrow_index, Ray::ONE);
        assert_eq!(step.supply_index, Ray::ONE);
        assert_eq!(step.revenue_shares, Ray::ZERO);
    }

    #[test]
    fn one_millisecond_moves_the_borrow_index_by_at_most_one_rate_unit() {
        let env = Env::default();
        let params = make_test_params(&env);
        let util = utilization(&env, Ray::from(60 * RAY), Ray::from(100 * RAY));
        let rate_per_ms = calculate_borrow_rate(&env, util, &params);
        let step = accrue_step(
            &env,
            &params,
            Ray::from(60 * RAY),
            Ray::from(100 * RAY),
            Ray::ONE,
            Ray::ONE,
            1,
        );
        let growth = step.borrow_index.raw() - RAY;
        let rate = rate_per_ms.raw();
        // The series adds x^2/2 on top of the linear term; nothing larger fits
        // in one millisecond.
        let quadratic = rate * rate / (2 * RAY);
        assert!(
            growth >= rate - 1 && growth - rate <= quadratic + 2,
            "growth {growth} vs rate {rate} (quadratic term {quadratic})"
        );
    }

    #[test]
    fn zero_borrowed_leaves_supply_index_and_revenue_untouched() {
        let env = Env::default();
        let params = make_test_params(&env);
        let step = accrue_step(
            &env,
            &params,
            Ray::ZERO,
            Ray::from(100 * RAY),
            Ray::ONE,
            Ray::ONE,
            MILLISECONDS_PER_YEAR,
        );
        assert_eq!(step.supply_index, Ray::ONE);
        assert_eq!(step.revenue_shares, Ray::ZERO);
        assert!(
            step.borrow_index.raw() > RAY,
            "the base rate compounds the borrow index even with no debt"
        );
    }

    #[test]
    fn zero_supplied_books_every_reward_as_revenue_shares() {
        let env = Env::default();
        let params = make_test_params(&env);
        let step = accrue_step(
            &env,
            &params,
            Ray::from(60 * RAY),
            Ray::ZERO,
            Ray::ONE,
            Ray::ONE,
            MILLISECONDS_PER_YEAR,
        );
        assert_eq!(step.supply_index, Ray::ONE, "no supplier to reward");
        assert!(
            step.revenue_shares.raw() > 0,
            "the accrued interest cannot vanish; it lands as revenue shares"
        );
    }

    #[test]
    fn at_the_borrow_index_ceiling_is_sticky_and_accrues_nothing() {
        let env = Env::default();
        let params = make_test_params(&env);
        let cap = Ray::from(crate::constants::MAX_BORROW_INDEX_RAY);
        let step = accrue_step(
            &env,
            &params,
            Ray::from(RAY),
            Ray::from(10 * RAY),
            cap,
            Ray::ONE,
            MILLISECONDS_PER_YEAR,
        );
        assert_eq!(step.borrow_index, cap);
        assert_eq!(step.supply_index, Ray::ONE);
        assert_eq!(step.revenue_shares, Ray::ZERO);
    }

    #[test]
    fn one_year_and_one_millisecond_are_two_chunks_in_the_simulator() {
        let env = Env::default();
        let sync = sample_sync(
            &env,
            PoolStateRaw {
                supplied: 100 * RAY,
                borrowed: 60 * RAY,
                revenue: 0,
                borrow_index: RAY,
                supply_index: RAY,
                last_timestamp: 0,
                cash: 40_000_000,
            },
        );
        let one_year = simulate_update_indexes(&env, MILLISECONDS_PER_YEAR, &sync);
        let one_year_plus = simulate_update_indexes(&env, MILLISECONDS_PER_YEAR + 1, &sync);
        assert!(one_year_plus.borrow_index > one_year.borrow_index);
        let extra = one_year_plus.borrow_index.raw() - one_year.borrow_index.raw();
        let params = make_test_params(&env);
        let rate_per_ms = calculate_borrow_rate(
            &env,
            utilization(&env, Ray::from(60 * RAY), Ray::from(100 * RAY)),
            &params,
        );
        let one_ms_of_growth = rate_per_ms.mul(&env, one_year.borrow_index).raw();
        assert!(
            extra <= 2 * one_ms_of_growth + 2,
            "the second chunk is one millisecond long: got {extra}, one ms is {one_ms_of_growth}"
        );
    }
}
