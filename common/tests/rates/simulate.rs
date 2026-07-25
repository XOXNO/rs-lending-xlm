//! Read-path accrual against an independent mirror.

use super::*;
use crate::constants::{MILLISECONDS_PER_YEAR, RAY, SUPPLY_INDEX_FLOOR_RAW};
use crate::rates::test_support::*;
use crate::types::PoolStateRaw;
use soroban_sdk::Env;

// Nonzero delta + live debt must accrue (not a no-op).
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

    // delta_ms > 0 accrues interest.
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

// Multi-year deltas use 1y chunks; chunked compound > single long Taylor eval.
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

    // Single Taylor evaluation across the full delta.
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
    // A 90%-utilization market over two years compounds past the
    // single-year index.
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
    // Query at the checkpoint timestamp: delta == 0 returns the stored indexes verbatim.
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
    // 100% reserve factor: no supplier rewards, supply_index stays at the floor;
    // all interest is fee and both paths reinvest it identically.
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

// Split accrual cannot lower indexes vs a single shot (no borrow-time farming).
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

    // Sub-chunk interval so the single call is exactly one Taylor evaluation.
    let total = MAX_COMPOUND_DELTA_MS / 2;
    let single = simulate_update_indexes(&env, total, &mk(RAY, RAY, 0));

    // Same interval, split at an arbitrary interior point (two Taylor evals).
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
    // Cadence alone must not double the index (loose runaway guard).
    assert!(
        split.borrow_index.raw() <= single.borrow_index.raw() * 2,
        "split ran away vs single: split={} single={}",
        split.borrow_index.raw(),
        single.borrow_index.raw()
    );
}
