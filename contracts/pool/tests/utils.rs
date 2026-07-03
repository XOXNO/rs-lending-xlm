extern crate std;

use super::*;
use crate::cache::Cache;
use crate::test_support::init_ledger;
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::RAY;
use common::math::fp::Ray;
use common::types::{HubAssetKey, MarketParamsRaw};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

/// Pool tests use hub 0 as a local fixture id.
fn hub(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

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
    Cache {
        env: env.clone(),
        supplied: Ray::from(supplied),
        borrowed: Ray::from(borrowed),
        revenue: Ray::ZERO,
        borrow_index: Ray::from(RAY),
        supply_index: Ray::from(RAY),
        last_timestamp: 0,
        current_timestamp: 1_000_000,
        params: params.into(),
        hub_asset: hub(&params.asset_id),
        cash,
    }
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
#[should_panic(expected = "Error(Contract, #")]
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
#[should_panic(expected = "Error(Contract, #")]
fn test_require_solvent_withdraw_state_panics_when_insolvent() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 0, RAY, 0);
        require_solvent_withdraw_state(&t.env, &cache);
    });
}

#[test]
fn test_apply_liquidation_fee_noop_when_not_liquidation_or_zero_fee() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = cache_with(&t.env, &t.params, 100 * RAY, 0, 50_000_000);
        let out = apply_liquidation_fee(&t.env, &mut cache, 10_000_000, false, 1_000_000);
        assert_eq!(out, 10_000_000);
        let out2 = apply_liquidation_fee(&t.env, &mut cache, 10_000_000, true, 0);
        assert_eq!(out2, 10_000_000);
    });
}

#[test]
fn test_apply_liquidation_fee_accrues_to_revenue_and_reduces_net() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = cache_with(&t.env, &t.params, 100 * RAY, 0, 50_000_000);
        let rev_before = cache.revenue;
        let net = apply_liquidation_fee(&t.env, &mut cache, 10_000_000, true, 2_000_000);
        assert_eq!(net, 8_000_000);
        assert!(cache.revenue > rev_before);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #")]
fn test_apply_liquidation_fee_rejects_fee_greater_than_gross() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = cache_with(&t.env, &t.params, 100 * RAY, 0, 50_000_000);
        let _ = apply_liquidation_fee(&t.env, &mut cache, 1_000_000, true, 2_000_000);
    });
}
