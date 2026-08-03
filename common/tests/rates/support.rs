use crate::constants::RAY;
use crate::math::fp::Ray;
use crate::rates::*;
use crate::types::{MarketParams, MarketParamsRaw, PoolStateRaw, PoolSyncData};
use soroban_sdk::{Address, Env};

pub const TEST_ASSET: &str = "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC";

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
