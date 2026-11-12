use crate::constants::{
    BPS, MAX_BORROW_INDEX_RAY, MAX_BORROW_RATE_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW,
};
use crate::errors::GenericError;
use crate::math::fp::Ray;
use crate::math::test_support::{contract_error_code, Sampler};
use crate::rates::*;
use crate::types::{MarketParams, MarketParamsRaw, PoolStateRaw, PoolSyncData};
use core::fmt::Debug;
use soroban_sdk::{Address, Env};

pub const TEST_ASSET: &str = "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC";

/// An environment with no budget limit, for tests that make many host calls.
pub fn unmetered_env() -> Env {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env
}

/// Checks `run` against the exact result: it returns `expected` when that is
/// representable, and panics with `MathOverflow` when it is `None`.
pub fn assert_exact<T: PartialEq + Debug>(
    env: &Env,
    expected: Option<T>,
    run: impl Fn(&Env) -> T,
    case: impl Debug,
) {
    match expected {
        Some(value) => assert_eq!(run(env), value, "{case:?}"),
        // A typed panic gets slower as the environment accumulates host
        // objects, so each expected panic runs in its own empty environment.
        None => assert_eq!(
            contract_error_code(|| {
                run(&Env::default());
            }),
            Some(GenericError::MathOverflow as u32),
            "{case:?}"
        ),
    }
}

/// Curve parameters that pass listing validation: `0 < mid < optimal < RAY`,
/// non-decreasing slopes, and a cap between the full-utilization rate and
/// `MAX_BORROW_RATE_RAY`.
pub fn sample_params(env: &Env, sampler: &mut Sampler) -> MarketParams {
    let base = sampler.at_most(RAY / 8);
    let slope1 = base + sampler.at_most(RAY / 8);
    let slope2 = slope1 + sampler.at_most(RAY / 8);
    let slope3 = slope2 + sampler.at_most(RAY / 2);
    let full = base + slope1 + slope2 + slope3;
    let mid = 1 + sampler.at_most(RAY - 3);
    let optimal = mid + 1 + sampler.at_most(RAY - mid - 2);
    MarketParams::from(&MarketParamsRaw {
        base_borrow_rate: base,
        slope1,
        slope2,
        slope3,
        max_borrow_rate: (full + sampler.at_most(MAX_BORROW_RATE_RAY - full)).max(base + 1),
        mid_utilization: mid,
        optimal_utilization: optimal,
        max_utilization: optimal + sampler.at_most(RAY - optimal),
        reserve_factor: sampler.below(BPS as u32),
        ..make_test_params_raw(env)
    })
}

/// An index between the supply-index floor and the index ceiling: a boundary
/// value, a value just above one, or any value in the range.
pub fn sample_index(sampler: &mut Sampler) -> i128 {
    const BOUNDARIES: [i128; 4] = [SUPPLY_INDEX_FLOOR_RAW, RAY, RAY + 1, MAX_BORROW_INDEX_RAY];
    match sampler.below(4) {
        0 => BOUNDARIES[sampler.below(4) as usize],
        1 => RAY + sampler.at_most(RAY),
        _ => {
            SUPPLY_INDEX_FLOOR_RAW + sampler.at_most(MAX_BORROW_INDEX_RAY - SUPPLY_INDEX_FLOOR_RAW)
        }
    }
}

/// The index one unit above one. For `shares` below `RAY`, the product
/// `shares * index / RAY` is `shares` plus the fraction `shares / RAY`.
pub const KNIFE_EDGE_INDEX: i128 = RAY + 1;

/// Share counts whose value at [`KNIFE_EDGE_INDEX`] is on, or one unit below,
/// a multiple of `unit` or a multiple plus half of `unit`, with the product's
/// fraction on each side of a half. Only at such values does the rounding of
/// the multiply change the result of a later division by `unit`, and random
/// draws almost never produce one. `unit` must divide `RAY / 10`.
pub fn knife_edge_shares(unit: i128) -> impl Iterator<Item = i128> {
    [RAY / 10 * 4, RAY / 10 * 6]
        .into_iter()
        .flat_map(move |multiple| {
            let half = multiple + unit / 2;
            [multiple - 1, multiple, half - 1, half]
        })
}

pub fn make_test_params_raw(env: &Env) -> MarketParamsRaw {
    MarketParamsRaw {
        base_borrow_rate: RAY / 100,
        slope1: RAY * 4 / 100,
        slope2: RAY * 10 / 100,
        slope3: RAY * 300 / 100,
        mid_utilization: RAY * 50 / 100,
        optimal_utilization: RAY * 80 / 100,
        max_utilization: RAY * 95 / 100,
        max_borrow_rate: RAY,
        reserve_factor: 1_000,
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: Address::from_str(env, TEST_ASSET),
        asset_decimals: 7,
    }
}

pub fn make_test_params(env: &Env) -> MarketParams {
    MarketParams::from(&make_test_params_raw(env))
}

pub fn sample_sync(env: &Env, state: PoolStateRaw) -> PoolSyncData {
    PoolSyncData {
        params: make_test_params_raw(env),
        state,
    }
}

pub fn oracle_accrual(
    env: &Env,
    params: &MarketParams,
    borrowed: Ray,
    mut supplied: Ray,
    mut borrow_index: Ray,
    mut supply_index: Ray,
    chunks_ms: &[u64],
) -> (Ray, Ray) {
    for &chunk in chunks_ms {
        let borrowed_orig = scaled_to_original(env, borrowed, borrow_index);
        let supplied_orig = scaled_to_original(env, supplied, supply_index);
        let util = utilization(env, borrowed_orig, supplied_orig);
        let rate = calculate_borrow_rate(env, util, params);
        let factor = compound_interest(env, rate, chunk);
        let new_borrow_index = update_borrow_index(env, borrow_index, factor);
        let (supplier_rewards, protocol_fee) =
            calculate_supplier_rewards(env, params, borrowed, new_borrow_index, borrow_index);
        let old_supply_index = supply_index;
        supply_index = update_supply_index(env, supplied, old_supply_index, supplier_rewards);
        let supplier_shortfall = supply_index_reward_shortfall(
            env,
            supplied,
            old_supply_index,
            supply_index,
            supplier_rewards,
        );
        borrow_index = new_borrow_index;

        let protocol_reward = protocol_fee.checked_add(env, supplier_shortfall);
        if protocol_reward != Ray::ZERO {
            let fee_scaled = protocol_fee_shares(env, protocol_reward, supply_index, supplied);
            supplied = supplied.checked_add(env, fee_scaled);
        }
    }
    (borrow_index, supply_index)
}
