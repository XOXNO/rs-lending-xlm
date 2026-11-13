use super::*;
use crate::constants::{
    MAX_BORROW_INDEX_RAY, MAX_MARKET_ENTRY_RAY, MAX_MARKET_VALUE_RAY, MAX_SUPPLY_INDEX_RAY,
    MILLISECONDS_PER_YEAR, RAY, SUPPLY_INDEX_FLOOR_RAW,
};
use crate::math::test_support::{half_up_of, Sampler};
use crate::rates::scaling::scaled_to_original;
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

/// One step never moves an index backwards or past its ceiling, and never
/// credits the supply side with more value than it charged the debt side.
/// Values are the half-up `shares * index` the pool reports, computed by the
/// exact reference, and the supply side includes the step's revenue shares.
#[test]
fn exact_accrual_step_never_credits_more_value_than_it_charges() {
    let env = unmetered_env();
    let value = |scaled: i128, index: i128| half_up_of(scaled, index, RAY).unwrap();
    let mut sampler = Sampler::new(71);
    for _ in 0..3_000 {
        let params = sample_params(&env, &mut sampler);
        let borrow_index = RAY + sampler.at_most(MAX_BORROW_INDEX_RAY - RAY);
        let supply_index = sample_index(&mut sampler);
        // One chunk grows debt by less than eight times, so a sixteenth of
        // the range keeps every value in the step inside `i128`.
        let borrowed = sampler.at_most(i128::MAX / 16 / (borrow_index / RAY + 1));
        let supplied = sampler.at_most(i128::MAX / 16 / (supply_index / RAY + 1));
        let delta_ms = sampler.at_most(i128::from(MAX_COMPOUND_DELTA_MS)) as u64;
        let case = (borrowed, supplied, borrow_index, supply_index, delta_ms);

        let step = accrue_step(
            &env,
            &params,
            Ray::from(borrowed),
            Ray::from(supplied),
            Ray::from(borrow_index),
            Ray::from(supply_index),
            delta_ms,
        );
        let (new_borrow_index, new_supply_index, revenue_shares) = (
            step.borrow_index.raw(),
            step.supply_index.raw(),
            step.revenue_shares.raw(),
        );
        assert!(
            (borrow_index..=MAX_BORROW_INDEX_RAY).contains(&new_borrow_index),
            "{case:?} under {params:?}: borrow index {new_borrow_index}"
        );
        assert!(
            (supply_index..=MAX_SUPPLY_INDEX_RAY).contains(&new_supply_index),
            "{case:?} under {params:?}: supply index {new_supply_index}"
        );
        assert!(revenue_shares >= 0, "{case:?} under {params:?}");

        let charged = value(borrowed, new_borrow_index) - value(borrowed, borrow_index);
        let credited =
            value(supplied + revenue_shares, new_supply_index) - value(supplied, supply_index);
        assert!(
            credited <= charged,
            "{case:?} under {params:?}: credited {credited}, charged {charged}"
        );
    }
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
        // Beyond the linear term, only x^2/2 is nonzero over one millisecond.
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

// --- accrue_step at the market value ceiling (audit finding S-04) ----------

mod accrue_step_value_ceiling {
    use super::*;
    use crate::math::fp::Ray;
    use crate::math::test_support::contract_error_code;

    /// Half-up value of `scaled` shares at `index`, or `None` past `i128`.
    fn value(scaled: i128, index: i128) -> Option<i128> {
        half_up_of(scaled, index, RAY)
    }

    fn step(
        env: &Env,
        params: &MarketParams,
        borrowed: i128,
        supplied: i128,
        borrow_index: i128,
        supply_index: i128,
        delta_ms: u64,
    ) -> AccrualStep {
        accrue_step(
            env,
            params,
            Ray::from(borrowed),
            Ray::from(supplied),
            Ray::from(borrow_index),
            Ray::from(supply_index),
            delta_ms,
        )
    }

    #[test]
    fn a_step_with_room_to_spare_is_not_capped() {
        let env = Env::default();
        let params = make_test_params(&env);
        let out = step(
            &env,
            &params,
            60 * RAY,
            100 * RAY,
            RAY,
            RAY,
            MILLISECONDS_PER_YEAR,
        );
        assert!(!out.value_capped);
        assert!(out.borrow_index.raw() > RAY);
        assert!(out.supply_index.raw() > RAY);
    }

    /// The audit's S-04 state: a market whose value is near `i128::MAX`. One
    /// year of interest would take the debt value past it; the step used to
    /// panic with `MathOverflow` and now holds the growth at the ceiling.
    #[test]
    fn a_step_that_would_cross_the_ceiling_is_held_at_it() {
        let env = Env::default();
        let params = make_test_params(&env);
        let room = 10 * RAY;
        let supplied = MAX_MARKET_VALUE_RAY - room;
        let borrowed = supplied / 10 * 9;
        let out = step(
            &env,
            &params,
            borrowed,
            supplied,
            RAY,
            RAY,
            MILLISECONDS_PER_YEAR,
        );
        assert!(out.value_capped, "the rate implies far more than the room");
        let (bi, si, revenue) = (
            out.borrow_index.raw(),
            out.supply_index.raw(),
            out.revenue_shares.raw(),
        );
        assert!(bi > RAY, "the room that was left is still accrued");
        let interest = value(borrowed, bi).unwrap() - borrowed;
        assert!(interest > 0 && interest <= room, "interest {interest}");
        // Almost all of the room is used: the index growth rounds down by
        // less than one raw unit per share of debt.
        assert!(room - interest <= borrowed / RAY + 1);
        assert!(value(borrowed, bi).unwrap() <= MAX_MARKET_VALUE_RAY);
        assert!(value(supplied + revenue, si).unwrap() <= MAX_MARKET_VALUE_RAY);
        assert!(si >= RAY && revenue >= 0);
    }

    #[test]
    fn a_step_at_the_ceiling_accrues_nothing() {
        let env = Env::default();
        let params = make_test_params(&env);
        for (borrowed, supplied) in [
            (MAX_MARKET_VALUE_RAY / 2, MAX_MARKET_VALUE_RAY),
            (MAX_MARKET_VALUE_RAY, MAX_MARKET_VALUE_RAY),
            // Past the ceiling, but still representable.
            (MAX_MARKET_VALUE_RAY / 2, i128::MAX),
        ] {
            let out = step(
                &env,
                &params,
                borrowed,
                supplied,
                RAY,
                RAY,
                MILLISECONDS_PER_YEAR,
            );
            assert!(out.value_capped);
            assert_eq!(out.borrow_index.raw(), RAY);
            assert_eq!(out.supply_index.raw(), RAY);
            assert_eq!(out.revenue_shares, Ray::ZERO);
        }
    }

    #[test]
    fn a_step_at_the_ceiling_without_debt_reports_no_cap() {
        let env = Env::default();
        let params = make_test_params(&env);
        let out = step(
            &env,
            &params,
            0,
            MAX_MARKET_VALUE_RAY,
            RAY,
            RAY,
            MILLISECONDS_PER_YEAR,
        );
        assert!(!out.value_capped, "there is no interest to hold");
        assert_eq!(out.supply_index.raw(), RAY);
        assert_eq!(out.revenue_shares, Ray::ZERO);
    }

    /// A total whose value does not fit `i128` at all holds the step instead
    /// of aborting it.
    #[test]
    fn a_step_with_an_unrepresentable_total_is_held() {
        let env = Env::default();
        let params = make_test_params(&env);
        for (borrowed, supplied, borrow_index, supply_index) in [
            (i128::MAX / 2, i128::MAX / 2, 4 * RAY, RAY),
            (RAY, i128::MAX / 2, RAY, 4 * RAY),
            (
                i128::MAX,
                i128::MAX,
                MAX_BORROW_INDEX_RAY,
                MAX_SUPPLY_INDEX_RAY,
            ),
        ] {
            assert!(
                value(borrowed, borrow_index).is_none() || value(supplied, supply_index).is_none()
            );
            let out = step(
                &env,
                &params,
                borrowed,
                supplied,
                borrow_index,
                supply_index,
                MILLISECONDS_PER_YEAR,
            );
            assert!(out.value_capped);
            assert_eq!(out.borrow_index.raw(), borrow_index);
            assert_eq!(out.supply_index.raw(), supply_index);
            assert_eq!(out.revenue_shares, Ray::ZERO);
        }
        let no_debt = step(
            &env,
            &params,
            0,
            i128::MAX / 2,
            RAY,
            4 * RAY,
            MILLISECONDS_PER_YEAR,
        );
        assert!(!no_debt.value_capped);
    }

    #[test]
    fn a_negative_total_or_index_still_panics() {
        let env = Env::default();
        let params = make_test_params(&env);
        for (borrowed, supplied, borrow_index, supply_index) in [
            (-1, RAY, RAY, RAY),
            (RAY, -1, RAY, RAY),
            (RAY, RAY, -1, RAY),
            (RAY, RAY, RAY, -1),
        ] {
            let code = contract_error_code(|| {
                let _ = step(
                    &env,
                    &params,
                    borrowed,
                    supplied,
                    borrow_index,
                    supply_index,
                    1,
                );
            });
            assert_eq!(code, Some(crate::errors::GenericError::MathOverflow as u32));
        }
    }

    /// Any non-negative totals and any stored indexes: the step returns, keeps
    /// both indexes monotone, and never takes a total across the ceiling.
    #[test]
    fn no_state_overflows_and_no_step_crosses_the_ceiling() {
        let env = unmetered_env();
        let mut sampler = Sampler::new(2_604);
        for round in 0..4_000 {
            let params = sample_params(&env, &mut sampler);
            let borrow_index = RAY + sampler.at_most(MAX_BORROW_INDEX_RAY - RAY);
            let supply_index = sample_index(&mut sampler);
            // A third of the cases sit near the ceiling, the rest anywhere.
            let (borrowed, supplied) = match round % 3 {
                0 => {
                    let near = |index: i128, sampler: &mut Sampler| {
                        let at_ceiling = Ray::from(MAX_MARKET_VALUE_RAY)
                            .div_floor_saturating(&env, Ray::from(index))
                            .raw();
                        at_ceiling - sampler.at_most(at_ceiling / 4)
                    };
                    (
                        near(borrow_index, &mut sampler),
                        near(supply_index, &mut sampler),
                    )
                }
                _ => (sampler.non_negative(), sampler.non_negative()),
            };
            let delta_ms = sampler.at_most(i128::from(MAX_COMPOUND_DELTA_MS)) as u64;
            let case = (borrowed, supplied, borrow_index, supply_index, delta_ms);

            let out = step(
                &env,
                &params,
                borrowed,
                supplied,
                borrow_index,
                supply_index,
                delta_ms,
            );
            let (bi, si, revenue) = (
                out.borrow_index.raw(),
                out.supply_index.raw(),
                out.revenue_shares.raw(),
            );
            assert!(
                (borrow_index..=MAX_BORROW_INDEX_RAY).contains(&bi),
                "{case:?}"
            );
            assert!(
                (supply_index..=MAX_SUPPLY_INDEX_RAY).contains(&si),
                "{case:?}"
            );
            assert!(
                revenue >= 0 && supplied.checked_add(revenue).is_some(),
                "{case:?}"
            );

            let (Some(debt), Some(supply)) =
                (value(borrowed, borrow_index), value(supplied, supply_index))
            else {
                assert_eq!(
                    (bi, si, revenue),
                    (borrow_index, supply_index, 0),
                    "{case:?}"
                );
                continue;
            };
            if debt.max(supply) >= MAX_MARKET_VALUE_RAY {
                // Without debt the borrow index still moves; it prices nothing.
                assert!(borrowed == 0 || bi == borrow_index, "{case:?}");
                assert_eq!((si, revenue), (supply_index, 0), "{case:?}");
                continue;
            }
            let new_debt = value(borrowed, bi).expect("debt value fits");
            let new_supply = value(supplied + revenue, si).expect("supply value fits");
            assert!(
                new_debt <= MAX_MARKET_VALUE_RAY,
                "{case:?}: debt {new_debt}"
            );
            assert!(
                new_supply <= MAX_MARKET_VALUE_RAY,
                "{case:?}: supply {new_supply}"
            );
            assert!(new_supply - supply <= new_debt - debt, "{case:?}");
        }
    }

    /// A market at the entry ceiling accrues a full year at the 200% maximum
    /// rate without reaching the value ceiling: the headroom factor is 8 and
    /// one year at that rate multiplies debt by less than `e^2`.
    #[test]
    fn the_entry_headroom_covers_a_year_at_the_maximum_rate() {
        let env = Env::default();
        let mut raw = make_test_params_raw(&env);
        raw.max_borrow_rate = 2 * RAY;
        raw.slope3 = 2 * RAY - raw.base_borrow_rate - raw.slope1 - raw.slope2;
        raw.verify(&env);
        let params = MarketParams::from(&raw);
        let out = step(
            &env,
            &params,
            MAX_MARKET_ENTRY_RAY,
            MAX_MARKET_ENTRY_RAY,
            RAY,
            RAY,
            MILLISECONDS_PER_YEAR,
        );
        assert!(!out.value_capped);
        let debt = value(MAX_MARKET_ENTRY_RAY, out.borrow_index.raw()).unwrap();
        assert!(debt > 7 * MAX_MARKET_ENTRY_RAY && debt < MAX_MARKET_VALUE_RAY);
    }

    /// Fifty years on a market at the entry ceiling: the simulator returns,
    /// and the debt value it implies stops at the ceiling.
    #[test]
    fn the_simulator_never_overflows_on_a_market_at_the_entry_ceiling() {
        let env = unmetered_env();
        let sync = sample_sync(
            &env,
            PoolStateRaw {
                supplied: MAX_MARKET_ENTRY_RAY,
                borrowed: MAX_MARKET_ENTRY_RAY / 10 * 9,
                revenue: 0,
                borrow_index: RAY,
                supply_index: RAY,
                last_timestamp: 0,
                cash: 0,
            },
        );
        let after = simulate_update_indexes(&env, 50 * MILLISECONDS_PER_YEAR, &sync);
        let debt = value(MAX_MARKET_ENTRY_RAY / 10 * 9, after.borrow_index.raw()).unwrap();
        assert!(debt <= MAX_MARKET_VALUE_RAY);
        assert!(
            debt > MAX_MARKET_VALUE_RAY / 10 * 9,
            "fifty years must reach the ceiling: {debt}"
        );
        let later = simulate_update_indexes(&env, 60 * MILLISECONDS_PER_YEAR, &sync);
        assert_eq!(
            later.borrow_index, after.borrow_index,
            "interest has stopped"
        );
    }
}
