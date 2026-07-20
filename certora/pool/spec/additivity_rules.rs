use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::RAY;
use common::types::{HubAssetKey, MarketParamsRaw, PoolKey, PoolStateRaw};

/// Hub-0 coordinate for `asset`; the spec models the single default hub.
fn hub(asset: Address) -> HubAssetKey {
    HubAssetKey { hub_id: 0, asset }
}

fn params(asset: Address) -> MarketParamsRaw {
    MarketParamsRaw {
        base_borrow_rate: RAY / 100,
        slope1: RAY / 10,
        slope2: RAY / 5,
        slope3: RAY / 2,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_borrow_rate: 2 * RAY,
        max_utilization: RAY,
        reserve_factor: 1_000,
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: asset,
        asset_decimals: 7,
    }
}

fn seed(env: &Env, admin: Address, asset: Address) {
    crate::LiquidityPool::__constructor(env.clone(), admin);
    env.storage()
        .persistent()
        .set(&PoolKey::Params(hub(asset.clone())), &params(asset.clone()));
    env.storage().persistent().set(
        &PoolKey::State(hub(asset)),
        &PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 25 * RAY,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: env.ledger().timestamp() * 1000,
            cash: 75 * RAY,
        },
    );
}

#[rule]
fn supply_split_scaled_amount_bounded_by_single(
    e: Env,
    admin: Address,
    asset: Address,
    x: i128,
    y: i128,
) {
    cvlr_assume!((0..=1_000_000_000_000i128).contains(&x));
    cvlr_assume!((0..=1_000_000_000_000i128).contains(&y));
    seed(&e, admin, asset.clone());

    let cache = crate::cache::Cache::load(&e, &hub(asset.clone()));
    let split = cache
        .calculate_scaled_supply(x)
        .checked_add(&e, cache.calculate_scaled_supply(y));
    let single = cache.calculate_scaled_supply(x + y);

    cvlr_assert!(split.raw() <= single.raw() + 2);
}

#[rule]
fn borrow_split_scaled_amount_bounded_by_single(
    e: Env,
    admin: Address,
    asset: Address,
    x: i128,
    y: i128,
) {
    cvlr_assume!((0..=1_000_000_000_000i128).contains(&x));
    cvlr_assume!((0..=1_000_000_000_000i128).contains(&y));
    seed(&e, admin, asset.clone());

    let cache = crate::cache::Cache::load(&e, &hub(asset.clone()));
    let split = cache
        .calculate_scaled_borrow(x)
        .checked_add(&e, cache.calculate_scaled_borrow(y));
    let single = cache.calculate_scaled_borrow(x + y);

    cvlr_assert!(split.raw() <= single.raw() + 2);
}

#[rule]
fn supply_withdraw_roundtrip_scaled_no_profit(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    seed(&e, admin, asset.clone());

    let cache = crate::cache::Cache::load(&e, &hub(asset.clone()));
    let scaled = cache.calculate_scaled_supply(amount);
    let recovered = cache.unscale_supply(scaled);

    cvlr_assert!(recovered <= amount + 1);
}

#[rule]
fn borrow_repay_roundtrip_scaled_no_profit(e: Env, admin: Address, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    seed(&e, admin, asset.clone());

    let cache = crate::cache::Cache::load(&e, &hub(asset.clone()));
    let scaled = cache.calculate_scaled_borrow(amount);
    let recovered = cache.unscale_borrow(scaled);

    cvlr_assert!(recovered <= amount + 1);
}

#[rule]
fn pool_additivity_reachability(e: Env, admin: Address, asset: Address) {
    seed(&e, admin, asset.clone());
    let cache = crate::cache::Cache::load(&e, &hub(asset.clone()));
    cvlr_satisfy!(cache.supplied.raw() > 0);
}
