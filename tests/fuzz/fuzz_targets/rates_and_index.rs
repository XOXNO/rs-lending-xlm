
#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, MAX_BORROW_RATE_RAY, MILLISECONDS_PER_YEAR, RAY};
use common::math::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_deposit_rate, calculate_supplier_rewards, compound_interest,
    simulate_update_indexes, MAX_COMPOUND_DELTA_MS,
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
}

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
        params: params_raw,
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
});
