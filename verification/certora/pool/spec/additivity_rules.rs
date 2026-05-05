use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::RAY;
use common::types::{MarketParams, PoolKey, PoolState};

fn params(asset: Address) -> MarketParams {
    MarketParams {
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 10,
        slope2_ray: RAY / 5,
        slope3_ray: RAY / 2,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY * 8 / 10,
        max_borrow_rate_ray: 2 * RAY,
        reserve_factor_bps: 1_000,
        asset_id: asset,
        asset_decimals: 7,
    }
}

fn seed(env: &Env, admin: Address, asset: Address) {
    crate::LiquidityPool::__constructor(env.clone(), admin, params(asset));
    env.storage().instance().set(
        &PoolKey::State,
        &PoolState {
            supplied_ray: 100 * RAY,
            borrowed_ray: 25 * RAY,
            revenue_ray: 0,
            borrow_index_ray: RAY,
            supply_index_ray: RAY,
            last_timestamp: env.ledger().timestamp() * 1000,
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
    seed(&e, admin, asset);

    let cache = crate::cache::Cache::load(&e);
    let split = cache.calculate_scaled_supply(x) + cache.calculate_scaled_supply(y);
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
    seed(&e, admin, asset);

    let cache = crate::cache::Cache::load(&e);
    let split = cache.calculate_scaled_borrow(x) + cache.calculate_scaled_borrow(y);
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
    seed(&e, admin, asset);

    let cache = crate::cache::Cache::load(&e);
    let scaled = cache.calculate_scaled_supply(amount);
    let recovered = cache.calculate_original_supply(scaled);

    cvlr_assert!(recovered <= amount + 1);
}

#[rule]
fn borrow_repay_roundtrip_scaled_no_profit(e: Env, admin: Address, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    seed(&e, admin, asset);

    let cache = crate::cache::Cache::load(&e);
    let scaled = cache.calculate_scaled_borrow(amount);
    let recovered = cache.calculate_original_borrow(scaled);

    cvlr_assert!(recovered <= amount + 1);
}

#[rule]
fn pool_additivity_reachability(e: Env, admin: Address, asset: Address) {
    seed(&e, admin, asset);
    let cache = crate::cache::Cache::load(&e);
    cvlr_satisfy!(cache.supplied.raw() > 0);
}
