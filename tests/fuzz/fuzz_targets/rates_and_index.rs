#![no_main]
use arbitrary::Arbitrary;
use common::constants::{
    BPS, MAX_BORROW_RATE_RAY, MAX_SUPPLY_INDEX_RAY, MILLISECONDS_PER_YEAR, RAY,
    SUPPLY_INDEX_FLOOR_RAW,
};
use common::math::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_deposit_rate, calculate_supplier_rewards, compound_interest,
    protocol_fee_shares, scaled_to_original, simulate_update_indexes,
    supply_index_reward_shortfall, update_borrow_index, update_supply_index, utilization,
    MAX_COMPOUND_DELTA_MS,
};
use common::types::{MarketParams, MarketParamsRaw, PoolStateRaw, PoolSyncData};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{Address, Env};

const MS_PER_YEAR: u64 = MILLISECONDS_PER_YEAR;

const MAX_ACCRUAL_MS: u64 = 10 * MS_PER_YEAR;

const AMOUNT_CAP_RAW: i128 = 100_000_000_000_000_000;

const START_BORROW_INDEX_GROWTH: i128 = 9 * RAY;
const BI_SCALE: i128 = START_BORROW_INDEX_GROWTH / (u64::MAX as i128);

const SUPPLY_INDEX_MIN_DIVISOR: i128 = 16;

const SWEEP_SUPPLIED_MAX: i128 = i128::MAX / (2 * (MAX_SUPPLY_INDEX_RAY / RAY));
const REWARD_CAP_RAW: i128 = i128::MAX / 4;
const INDEX_SPAN_PER_UNIT: i128 = (MAX_SUPPLY_INDEX_RAY - SUPPLY_INDEX_FLOOR_RAW) / (u64::MAX as i128);

#[derive(Debug, Arbitrary)]
struct In {
    util_bps: u16,
    base_pct: u8,
    s1_pct: u8,
    s2_pct: u8,
    s3_pct: u16,
    mid_pct: u8,
    opt_pct: u8,
    max_pct: u16,
    max_util_pct: u8,
    reserve_pct: u8,

    delta_ms: u64,
    borrowed_units: u64,
    supplied_units: u64,
    chunk_units: u64,
    borrow_index_units: u64,
    supply_index_units: u64,

    dust_supplied_units: u64,
    dust_old_index_units: u64,
    dust_reward_hi: u64,
    dust_reward_lo: u64,

    /// How many chunks the accrual span is split into (see `PARTITION_MAX`).
    partition_count: u8,
    /// Relative chunk widths; only the first `n` entries are used.
    partition_weights: [u16; PARTITION_MAX],
}

/// Upper bound on partition chunks. Each chunk costs one full compounding step
/// (plus one per `MAX_COMPOUND_DELTA_MS` it spans), so this bounds per-iteration
/// cost while still covering uneven splits.
const PARTITION_MAX: usize = 8;

/// Longest span used by the partition property. Two years keeps every chunk to
/// at most two internal compounding steps.
const PARTITION_SPAN_MAX_MS: u64 = 2 * MS_PER_YEAR;

fn make_params(env: &Env, i: &In) -> MarketParamsRaw {
    let cap = MAX_BORROW_RATE_RAY;

    let base = cap * (i.base_pct as i128) / 1_024;
    let s1 = base + (cap - base) * (i.s1_pct as i128) / 256;
    let s2 = s1 + (cap - s1) * (i.s2_pct as i128) / 256;
    let s3 = s2 + (cap - s2) * (i.s3_pct as i128) / 65_536;

    let max_rate = (s3 + (cap - s3) * (i.max_pct as i128) / 65_536).max(base + 1);

    let mid = RAY * (i.mid_pct as i128 % 98 + 1) / 100;
    let optimal = mid + (RAY - mid) * (i.opt_pct as i128 % 99 + 1) / 101;
    let max_util = optimal + (RAY - optimal) * (i.max_util_pct as i128) / 256;

    MarketParamsRaw {
        max_borrow_rate: max_rate,
        base_borrow_rate: base,
        slope1: s1,
        slope2: s2,
        slope3: s3,
        mid_utilization: mid,
        optimal_utilization: optimal,
        max_utilization: max_util,

        reserve_factor: (i.reserve_pct as u32) * (BPS as u32 - 1) / (u8::MAX as u32),
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: Address::from_str(
            env,
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
        ),
        asset_decimals: 7,
    }
}

fn assert_rate_invariants(env: &Env, util_bps: i128, params: &MarketParams, rate: Ray) {
    assert!(
        rate.raw() >= 0,
        "negative rate at util_bps={} mid={}",
        util_bps,
        params.mid_utilization.raw()
    );

    let max_per_ms = params.max_borrow_rate.raw() / MS_PER_YEAR as i128;
    assert!(
        rate.raw() <= max_per_ms + 2,
        "rate exceeded max: rate={} max_per_ms={}",
        rate.raw(),
        max_per_ms
    );

    if util_bps < 10_000 {
        let util_hi = Ray::from(RAY * (util_bps + 1) / 10_000);
        let rate_hi = calculate_borrow_rate(env, util_hi, params);
        assert!(
            rate.raw() <= rate_hi.raw() + 2,
            "monotonicity violated: rate({})={} > rate({})={}",
            util_bps,
            rate.raw(),
            util_bps + 1,
            rate_hi.raw()
        );
    }
}

fn assert_compound_invariants(env: &Env, rate: Ray, delta_ms: u64, factor: Ray) {
    if delta_ms == 0 {
        assert_eq!(factor, Ray::ONE);
        return;
    }

    assert!(
        factor.raw() >= RAY - 1,
        "compound below 1.0: factor={} rate={} dt={}",
        factor.raw(),
        rate.raw(),
        delta_ms
    );

    let prev = compound_interest(env, rate, delta_ms - 1);
    assert!(
        factor.raw() >= prev.raw(),
        "non-monotonic in delta: f(t-1)={} f(t)={}",
        prev.raw(),
        factor.raw()
    );

    if let Some(rt) = rate.raw().checked_mul(delta_ms as i128) {
        if let Some(linear_floor) = RAY.checked_add(rt) {
            assert!(
                factor.raw() >= linear_floor - 2,
                "Taylor below linear floor: factor={} 1+r*t={}",
                factor.raw(),
                linear_floor
            );
        }
    }
}

fn assert_interest_split(
    env: &Env,
    params: &MarketParams,
    borrowed: Ray,
    new_index: Ray,
    old_index: Ray,
) {
    let (rewards, fee) = calculate_supplier_rewards(env, params, borrowed, new_index, old_index);

    assert!(
        rewards.raw() >= 0 && fee.raw() >= 0,
        "negative split: rewards={} fee={}",
        rewards.raw(),
        fee.raw()
    );

    let old_debt = borrowed.mul(env, old_index);
    let new_debt = borrowed.mul(env, new_index);
    let accrued = new_debt.checked_sub(env, old_debt);

    assert_eq!(
        rewards.raw() + fee.raw(),
        accrued.raw(),
        "§5 conservation: rewards={} + fee={} != accrued={}",
        rewards.raw(),
        fee.raw(),
        accrued.raw()
    );

    if params.reserve_factor.raw() == 0 {
        assert_eq!(fee.raw(), 0, "fee non-zero with reserve_factor=0");
    }

    if let (Some(fee_scaled), Some(rf_scaled)) = (
        fee.raw().checked_mul(BPS),
        params.reserve_factor.raw().checked_mul(accrued.raw()),
    ) {
        let err = (fee_scaled - rf_scaled).abs();
        assert!(
            err <= BPS / 2 + 1,
            "§5 fee rounding: fee*BPS={} rf*accrued={} err={} rf={}",
            fee_scaled,
            rf_scaled,
            err,
            params.reserve_factor.raw()
        );
    }
}

fn assert_no_dust_inflation(env: &Env, supplied: Ray, old_index: Ray, rewards: Ray) {
    let new_index = update_supply_index(env, supplied, old_index, rewards);

    assert!(
        new_index.raw() >= old_index.raw(),
        "index regressed: supplied={} old={} rewards={} new={}",
        supplied.raw(),
        old_index.raw(),
        rewards.raw(),
        new_index.raw()
    );

    assert!(
        new_index.raw() <= MAX_SUPPLY_INDEX_RAY,
        "index above cap: supplied={} old={} rewards={} new={}",
        supplied.raw(),
        old_index.raw(),
        rewards.raw(),
        new_index.raw()
    );

    let distributed = supplied
        .mul(env, new_index)
        .checked_sub(env, supplied.mul(env, old_index));
    assert!(
        distributed.raw() <= rewards.raw(),
        "dust inflation: supplied={} old={} rewards={} distributed={}",
        supplied.raw(),
        old_index.raw(),
        rewards.raw(),
        distributed.raw()
    );

    let shortfall = supply_index_reward_shortfall(env, supplied, old_index, new_index, rewards);
    assert_eq!(
        distributed.raw() + shortfall.raw(),
        rewards.raw(),
        "reward not conserved: supplied={} old={} rewards={} distributed={} shortfall={}",
        supplied.raw(),
        old_index.raw(),
        rewards.raw(),
        distributed.raw(),
        shortfall.raw()
    );
}

/// Everything one accrual run carries forward. `simulate_update_indexes`
/// returns only the two indexes, so it cannot be composed chunk-by-chunk:
/// `supplied` grows as protocol-revenue shares are minted, and the next chunk's
/// utilization and index growth both depend on it.
#[derive(Clone, Copy)]
struct AccrualState {
    borrow_index: Ray,
    supply_index: Ray,
    supplied: Ray,
}

/// Mirrors `simulate_update_indexes_body` using only public `common::rates`
/// helpers, so a span can be accrued in pieces.
///
/// `assert_partition_invariants` pins this against the production entry point
/// on the single-chunk case before using it, so it cannot silently drift.
fn accrue_span(
    env: &Env,
    params: &MarketParams,
    borrowed: Ray,
    state: AccrualState,
    span_ms: u64,
) -> AccrualState {
    let mut st = state;
    let mut remaining = span_ms;
    while remaining > 0 {
        let chunk = core::cmp::min(remaining, MAX_COMPOUND_DELTA_MS);

        let borrowed_original = scaled_to_original(env, borrowed, st.borrow_index);
        let supplied_original = scaled_to_original(env, st.supplied, st.supply_index);
        let util = utilization(env, borrowed_original, supplied_original);
        let borrow_rate = calculate_borrow_rate(env, util, params);
        let interest_factor = compound_interest(env, borrow_rate, chunk);

        let new_borrow_index = update_borrow_index(env, st.borrow_index, interest_factor);
        let (supplier_rewards, protocol_fee) =
            calculate_supplier_rewards(env, params, borrowed, new_borrow_index, st.borrow_index);

        let old_supply_index = st.supply_index;
        st.supply_index = update_supply_index(env, st.supplied, old_supply_index, supplier_rewards);
        let shortfall = supply_index_reward_shortfall(
            env,
            st.supplied,
            old_supply_index,
            st.supply_index,
            supplier_rewards,
        );
        st.borrow_index = new_borrow_index;

        let protocol_reward = protocol_fee.checked_add(env, shortfall);
        if protocol_reward != Ray::ZERO {
            let fee_scaled =
                protocol_fee_shares(env, protocol_reward, st.supply_index, st.supplied);
            st.supplied = st.supplied.checked_add(env, fee_scaled);
        }

        remaining -= chunk;
    }
    st
}

/// Upper bound on the borrow index after `chunks`: the same compounding walk
/// driven by `params.max_borrow_rate` instead of the curve. `calculate_borrow_rate`
/// is capped at that rate and both `compound_interest` and `update_borrow_index`
/// are monotone in their inputs, so this dominates any real run exactly.
fn max_borrow_index_after(
    env: &Env,
    params: &MarketParams,
    start_index: Ray,
    chunks: &[u64],
) -> Ray {
    let max_rate = params.max_borrow_rate.div_by_int(env, MS_PER_YEAR as i128);
    let mut index = start_index;
    for &chunk in chunks {
        let mut remaining = chunk;
        while remaining > 0 {
            let step = core::cmp::min(remaining, MAX_COMPOUND_DELTA_MS);
            index = update_borrow_index(env, index, compound_interest(env, max_rate, step));
            remaining -= step;
        }
    }
    index
}

/// Splits `total_ms` into `n` chunks whose widths follow `weights`. The chunks
/// sum to exactly `total_ms`; individual chunks may be zero.
fn partition(
    total_ms: u64,
    weights: &[u16; PARTITION_MAX],
    n: usize,
) -> ([u64; PARTITION_MAX], usize) {
    let mut chunks = [0u64; PARTITION_MAX];
    let total_weight: u128 = weights[..n].iter().map(|w| *w as u128 + 1).sum();

    let mut acc_weight: u128 = 0;
    let mut assigned: u64 = 0;
    for k in 0..n {
        acc_weight += weights[k] as u128 + 1;
        let cut = (total_ms as u128 * acc_weight / total_weight) as u64;
        chunks[k] = cut - assigned;
        assigned = cut;
    }
    debug_assert_eq!(assigned, total_ms);
    (chunks, n)
}

/// `update_indexes` is permissionless, so the caller chooses how a span is
/// partitioned into accruals. CS-AAVE4-004 is exactly a partition that strands
/// value: Aave V4 floored the fee to zero once accrual ran every second, so the
/// interest borrowers paid stopped reaching anyone.
///
/// The property asserted here is therefore **per path**: whatever the partition,
/// the value credited to suppliers plus treasury must equal the interest charged
/// to borrowers, up to a bounded per-compounding-step rounding residual, and must
/// never exceed it.
///
/// Deliberately NOT asserted: that a partitioned run leaves suppliers (or
/// suppliers plus treasury) with at least as much as a single terminal accrual.
/// That cross-path comparison is false in this target's parameter domain and the
/// counterexamples are not leaks:
///
/// * Protocol fee shares minted by an early chunk compound for the rest of the
///   span, so a partitioned run shifts value from suppliers to the treasury.
///   With a 90%+ reserve factor and a ~200% APR left un-accrued for two years,
///   the original suppliers can end with ~21% of the single-accrual claim while
///   suppliers+treasury still *grows*. Deferring the mint is what over-credits
///   suppliers; frequent accrual is the economically correct side.
/// * A partitioned run re-evaluates utilization more often and can therefore
///   settle on a *lower* rate trajectory, charging borrowers less interest and
///   so booking less value in total. Charging less is not destroying value.
///
/// `contracts/pool/tests/interest.rs` asserts the cross-path directional
/// property unconditionally for realistic markets (a ~$1M book at ~10% APR,
/// 10% reserve factor), where it holds with a positive margin at every cadence
/// down to one accrual per second.
fn assert_partition_invariants(
    env: &Env,
    params_raw: &MarketParamsRaw,
    params: &MarketParams,
    borrowed: Ray,
    start: AccrualState,
    total_ms: u64,
    input: &In,
) {
    let single = accrue_span(env, params, borrowed, start, total_ms);

    // Pin the local model to the production read path.
    let sync = PoolSyncData {
        params: params_raw.clone(),
        state: PoolStateRaw {
            supplied: start.supplied.raw(),
            borrowed: borrowed.raw(),
            revenue: 0,
            cash: 0,
            borrow_index: start.borrow_index.raw(),
            supply_index: start.supply_index.raw(),
            last_timestamp: 0,
        },
    };
    let production = simulate_update_indexes(env, total_ms, &sync);
    assert_eq!(
        single.borrow_index.raw(),
        production.borrow_index.raw(),
        "model drift: borrow index {} != production {} (dt={})",
        single.borrow_index.raw(),
        production.borrow_index.raw(),
        total_ms
    );
    assert_eq!(
        single.supply_index.raw(),
        production.supply_index.raw(),
        "model drift: supply index {} != production {} (dt={})",
        single.supply_index.raw(),
        production.supply_index.raw(),
        total_ms
    );

    let n = (input.partition_count as usize % (PARTITION_MAX - 1)) + 2;
    let (chunks, n) = partition(total_ms, &input.partition_weights, n);

    let mut part = start;
    let mut part_steps: i128 = 0;
    for &chunk in &chunks[..n] {
        part = accrue_span(env, params, borrowed, part, chunk);
        part_steps += compounding_steps(chunk);
    }

    // The rate curve is capped at `max_borrow_rate`, so neither path can compound
    // past the ceiling that rate would produce over the same chunk boundaries.
    let part_ceiling = max_borrow_index_after(env, params, start.borrow_index, &chunks[..n]);
    assert!(
        part.borrow_index.raw() <= part_ceiling.raw(),
        "partitioned borrow index above the max-rate ceiling: part={} ceiling={} n={} dt={}",
        part.borrow_index.raw(),
        part_ceiling.raw(),
        n,
        total_ms
    );
    let single_ceiling = max_borrow_index_after(env, params, start.borrow_index, &[total_ms]);
    assert!(
        single.borrow_index.raw() <= single_ceiling.raw(),
        "single-shot borrow index above the max-rate ceiling: single={} ceiling={} dt={}",
        single.borrow_index.raw(),
        single_ceiling.raw(),
        total_ms
    );

    for (label, st, steps) in [
        ("single", &single, compounding_steps(total_ms)),
        ("partitioned", &part, part_steps),
    ] {
        assert_interest_reaches_someone(env, borrowed, start, st, steps, label, total_ms);
    }
}

/// Number of `MAX_COMPOUND_DELTA_MS` compounding steps `accrue_span` performs
/// for a span of `span_ms`. Zero for a zero-length span.
fn compounding_steps(span_ms: u64) -> i128 {
    span_ms.div_ceil(MAX_COMPOUND_DELTA_MS) as i128
}

/// The no-leak invariant: interest charged to borrowers must land with suppliers
/// or the treasury, never be minted out of nothing and never be stranded beyond a
/// bounded per-step rounding residual.
///
/// Per compounding step the accrual can strand at most:
///
/// * under one ray of supply index (`update_supply_index` floors), worth
///   `supplied / RAY` in value; that part is re-booked to the treasury by
///   `supply_index_reward_shortfall`, so it is normally not stranded at all; and
/// * under one scaled share of protocol fee (`protocol_fee_shares` floors),
///   worth `supply_index / RAY` in value.
///
/// The supply index never decreases during accrual, so the terminal index bounds
/// every step's contribution.
///
/// Both directions have to scale with `steps`. Measuring both sides with
/// `mul_floor` does NOT make the comparison rounding-neutral, which is what an
/// earlier flat `-1` lower bound assumed: production accrues with `Ray::mul`
/// (`mul_div_half_up`, see common/src/math/fp.rs), so each step's half-up
/// rounding is already baked into the indexes these floors are applied to. A
/// step can therefore push `credited` a unit past the floor-measured `charged`
/// without any interest being minted, and over enough steps that accumulates
/// past any constant. Run 31856319201 hit exactly this on the partitioned path
/// (charged=4 credited=6 over a ~2 year span).
fn assert_interest_reaches_someone(
    env: &Env,
    borrowed: Ray,
    start: AccrualState,
    end: &AccrualState,
    steps: i128,
    label: &str,
    total_ms: u64,
) {
    let charged = borrowed.mul_floor(env, end.borrow_index).raw()
        - borrowed.mul_floor(env, start.borrow_index).raw();
    let credited = end.supplied.mul_floor(env, end.supply_index).raw()
        - start.supplied.mul_floor(env, start.supply_index).raw();
    let residual = charged - credited;

    // One unit per compounding step for production's half-up rounding, plus one
    // for the terminal pair of measurement floors. Deliberately far tighter than
    // the stranding bound below, which carries a `supplied / RAY` term: minting
    // is the dangerous direction, and reusing that term here would let a real
    // over-credit of nearly any size pass. If fuzzing ever exceeds this, treat it
    // as a finding rather than a bound to widen.
    let mint_slack = steps.saturating_add(1);
    assert!(
        residual >= -mint_slack,
        "[{label}] credited {} more than borrowers were charged over {steps} \
         compounding steps (slack {mint_slack}): charged={charged} \
         credited={credited} dt={total_ms} supplied={} supply_index={}",
        -residual,
        start.supplied.raw(),
        end.supply_index.raw()
    );

    let bound = steps.saturating_mul(end.supply_index.raw() / RAY + start.supplied.raw() / RAY + 4);
    assert!(
        residual <= bound,
        "[{label}] stranded {residual} ray of interest over {steps} compounding steps \
         (bound {bound}): charged={charged} credited={credited} dt={total_ms} \
         supplied={} supply_index={}",
        start.supplied.raw(),
        end.supply_index.raw()
    );
}

fuzz_target!(|i: In| {
    let env = Env::default();

    let params_raw = make_params(&env, &i);
    params_raw.verify(&env);
    let params = MarketParams::from(&params_raw);

    let util_bps = (i.util_bps % 10_001) as i128;
    let util = Ray::from(RAY * util_bps / 10_000);
    let rate = calculate_borrow_rate(&env, util, &params);
    assert_rate_invariants(&env, util_bps, &params, rate);

    let chunk_ms = i.chunk_units % (MAX_COMPOUND_DELTA_MS + 1);
    let factor = compound_interest(&env, rate, chunk_ms);
    assert_compound_invariants(&env, rate, chunk_ms, factor);

    let borrowed_raw = (i.borrowed_units as i128 % AMOUNT_CAP_RAW) + 1;
    assert_interest_split(&env, &params, Ray::from(borrowed_raw), factor, Ray::ONE);

    let deposit_rate = calculate_deposit_rate(&env, util, rate, params.reserve_factor);
    assert!(
        deposit_rate.raw() >= 0,
        "negative deposit rate: {}",
        deposit_rate.raw()
    );

    assert!(
        deposit_rate.raw() <= rate.raw() + 1,
        "deposit rate > borrow rate: dep={} bor={}",
        deposit_rate.raw(),
        rate.raw()
    );
    if params.reserve_factor.raw() == 0 && util.raw() > 0 {

        let expected = rate.mul(&env, util);
        let diff = (deposit_rate.raw() - expected.raw()).abs();
        assert!(
            diff <= 1,
            "deposit rate mismatch with rf=0: dep={} expected={} diff={}",
            deposit_rate.raw(),
            expected.raw(),
            diff
        );
    }

    let supplied_raw = borrowed_raw + 1 + (i.supplied_units as i128 % AMOUNT_CAP_RAW);
    let total_delta_ms = i.delta_ms % (MAX_ACCRUAL_MS + 1);

    let start_borrow_index = RAY + i.borrow_index_units as i128 * BI_SCALE;
    let si_floor = start_borrow_index / SUPPLY_INDEX_MIN_DIVISOR;
    let si_scale = (start_borrow_index - si_floor) / (u64::MAX as i128);
    let start_supply_index = si_floor + i.supply_index_units as i128 * si_scale;

    let sync = PoolSyncData {
        params: params_raw.clone(),
        state: PoolStateRaw {
            supplied: supplied_raw,
            borrowed: borrowed_raw,
            revenue: 0,
            cash: 0,
            borrow_index: start_borrow_index,
            supply_index: start_supply_index,
            last_timestamp: 0,
        },
    };
    let new_idx = simulate_update_indexes(&env, total_delta_ms, &sync);

    assert!(
        new_idx.borrow_index.raw() >= start_borrow_index,
        "borrow index regressed: new={} old={} dt={}",
        new_idx.borrow_index.raw(),
        start_borrow_index,
        total_delta_ms
    );
    assert!(
        new_idx.supply_index.raw() >= start_supply_index,
        "supply index regressed: new={} old={} dt={}",
        new_idx.supply_index.raw(),
        start_supply_index,
        total_delta_ms
    );

    if total_delta_ms == 0 {
        assert_eq!(new_idx.borrow_index.raw(), start_borrow_index);
        assert_eq!(new_idx.supply_index.raw(), start_supply_index);
    }

    assert_partition_invariants(
        &env,
        &params_raw,
        &params,
        Ray::from(borrowed_raw),
        AccrualState {
            borrow_index: Ray::from(start_borrow_index),
            supply_index: Ray::from(start_supply_index),
            supplied: Ray::from(supplied_raw),
        },
        total_delta_ms.min(PARTITION_SPAN_MAX_MS),
        &i,
    );

    let dust_supplied = Ray::from(1 + (i.dust_supplied_units as i128 % SWEEP_SUPPLIED_MAX));
    let dust_old_index =
        Ray::from(SUPPLY_INDEX_FLOOR_RAW + i.dust_old_index_units as i128 * INDEX_SPAN_PER_UNIT);
    let reward_wide = ((i.dust_reward_hi as u128) << 64) | i.dust_reward_lo as u128;
    let dust_reward = Ray::from((reward_wide % (REWARD_CAP_RAW as u128)) as i128);
    assert_no_dust_inflation(&env, dust_supplied, dust_old_index, dust_reward);
});
