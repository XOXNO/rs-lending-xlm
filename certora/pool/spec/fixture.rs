//! Shared synchronized market fixture for pool-core accounting rules.
#![allow(dead_code)]

use common::constants::RAY;
use common::types::{
    HubAssetKey, MarketParamsRaw, PoolAction, PoolKey, PoolStateRaw, ScaledPositionRaw,
};
use soroban_sdk::{Address, Env};

pub const ASSET_DECIMALS: u32 = 7;
pub const ONE_TOKEN: i128 = 10_000_000;
pub const MAX_FLOW_AMOUNT: i128 = 100 * ONE_TOKEN;

pub fn hub(asset: Address) -> HubAssetKey {
    HubAssetKey { hub_id: 0, asset }
}

pub fn params(asset: Address, flashloan_fee: u32, is_flashloanable: bool) -> MarketParamsRaw {
    MarketParamsRaw {
        base_borrow_rate: RAY / 100,
        slope1: RAY / 10,
        slope2: RAY / 5,
        slope3: RAY / 2,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_borrow_rate: 2 * RAY,
        // RAY is the validated sentinel that disables the operation-level cap;
        // the rate/index rules verify the utilization curve separately.
        max_utilization: RAY,
        reserve_factor: 1_000,
        is_flashloanable,
        flashloan_fee,
        asset_id: asset,
        asset_decimals: ASSET_DECIMALS,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn state(
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    borrow_index: i128,
    supply_index: i128,
    cash: i128,
    timestamp_seconds: u64,
) -> PoolStateRaw {
    PoolStateRaw {
        supplied,
        borrowed,
        revenue,
        borrow_index,
        supply_index,
        last_timestamp: timestamp_seconds * 1_000,
        cash,
    }
}

pub fn seed(
    env: &Env,
    admin: Address,
    asset: Address,
    params: MarketParamsRaw,
    state: PoolStateRaw,
) {
    crate::LiquidityPool::__constructor(env.clone(), admin);
    let key = hub(asset);
    env.storage()
        .persistent()
        .set(&PoolKey::Params(key.clone()), &params);
    env.storage().persistent().set(&PoolKey::State(key), &state);
}

pub fn read_state(env: &Env, asset: &Address) -> PoolStateRaw {
    env.storage()
        .persistent()
        .get(&PoolKey::State(hub(asset.clone())))
        .unwrap()
}

pub fn position(scaled_amount: i128) -> ScaledPositionRaw {
    ScaledPositionRaw { scaled_amount }
}

pub fn action(asset: Address, scaled_amount: i128, amount: i128) -> PoolAction {
    PoolAction {
        position: position(scaled_amount),
        amount,
        hub_asset: hub(asset),
    }
}
