extern crate std;

use super::*;
use crate::cache::Cache;
use crate::test_support::{hub, init_ledger};
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::RAY;
use common::types::{MarketParamsRaw, PoolStateRaw};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

struct TestSetup {
    env: Env,
    contract: Address,
    params: MarketParamsRaw,
}

impl TestSetup {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        init_ledger(&env);

        let admin = Address::generate(&env);
        let asset = Address::generate(&env);
        let params = MarketParamsRaw {
            max_borrow_rate: 2 * RAY,
            base_borrow_rate: RAY / 100,
            slope1: RAY / 10,
            slope2: RAY / 5,
            slope3: RAY / 2,
            mid_utilization: RAY / 2,
            optimal_utilization: RAY * 8 / 10,
            max_utilization: RAY * 95 / 100,
            reserve_factor: 1_000,
            is_flashloanable: false,
            flashloan_fee: 0,
            asset_id: asset.clone(),
            asset_decimals: 7,
        };
        let contract = env.register(LiquidityPool, (admin.clone(),));
        LiquidityPoolClient::new(&env, &contract).create_market(&0u32, &params);

        Self {
            env,
            contract,
            params,
        }
    }

    fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
        self.env.as_contract(&self.contract, f)
    }
}

fn cache_with(
    env: &Env,
    params: &MarketParamsRaw,
    supplied: i128,
    borrowed: i128,
    cash: i128,
) -> Cache {
    Cache::from_parts(
        env,
        hub(&params.asset_id),
        params,
        &PoolStateRaw {
            supplied,
            borrowed,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash,
        },
        1_000_000,
    )
}

#[test]
fn test_require_utilization_below_max_early_returns_when_supplied_zero() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 0, 100 * RAY, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

#[test]
fn test_require_utilization_below_max_early_returns_when_max_util_ge_one() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut params = t.params.clone();
        params.max_utilization = RAY;
        let cache = cache_with(&t.env, &params, 10 * RAY, 11 * RAY, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #127)")]
fn test_require_utilization_below_max_panics_when_above() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 10 * RAY, 10 * RAY, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

#[test]
fn test_require_solvent_withdraw_state_happy() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 10 * RAY, 5 * RAY, 0);
        require_solvent_withdraw_state(&t.env, &cache);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #123)")]
fn test_require_solvent_withdraw_state_panics_when_insolvent() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 0, RAY, 0);
        require_solvent_withdraw_state(&t.env, &cache);
    });
}

/// Cash the buffer holds back for a given cache, in asset units.
fn reserved_for(env: &Env, cache: &Cache) -> i128 {
    let supplied = cache.unscale_supply_floor(cache.supplied());
    common::math::fp::Bps::from(common::constants::LIQUIDATION_BUFFER_BPS).apply_to(env, supplied)
}

/// An ordinary draw may take the market down to the buffer, but not through it.
#[test]
fn test_require_liquidation_buffer_admits_a_draw_down_to_the_reserve() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 1_000 * RAY, 0, 1_000);
        let reserved = reserved_for(&t.env, &cache);
        assert!(
            reserved > 0,
            "fixture must reserve something to be meaningful"
        );
        require_liquidation_buffer(&t.env, &cache, 1_000 - reserved);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #112)")]
fn test_require_liquidation_buffer_rejects_a_draw_one_unit_past_the_reserve() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 1_000 * RAY, 0, 1_000);
        let reserved = reserved_for(&t.env, &cache);
        require_liquidation_buffer(&t.env, &cache, 1_000 - reserved + 1);
    });
}
