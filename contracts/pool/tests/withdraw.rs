extern crate std;

use super::withhold_liquidation_fee;
use crate::cache::Cache;
use crate::test_support::{hub, init_ledger};
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::RAY;
use common::math::fp::Ray;
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

    fn cache(&self, supplied: i128, cash: i128) -> Cache {
        Cache::from_parts(
            &self.env,
            hub(&self.params.asset_id),
            &self.params,
            &PoolStateRaw {
                supplied,
                borrowed: 0,
                revenue: 0,
                borrow_index: RAY,
                supply_index: RAY,
                last_timestamp: 0,
                cash,
            },
            1_000_000,
        )
    }
}

#[test]
fn test_withhold_liquidation_fee_noop_when_not_liquidation_or_zero_fee() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.cache(100 * RAY, 50_000_000);
        let out = withhold_liquidation_fee(&t.env, &mut cache, 10_000_000, false, 1_000_000);
        assert_eq!(out, 10_000_000);
        let out2 = withhold_liquidation_fee(&t.env, &mut cache, 10_000_000, true, 0);
        assert_eq!(out2, 10_000_000);
    });
}

#[test]
fn test_withhold_liquidation_fee_accrues_to_revenue_and_reduces_net() {
    let env = Env::default();
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.cache(100 * RAY, 50_000_000);
        let fee_raw = 2_000_000i128;

        let expected_revenue = Ray::from_asset(&env, fee_raw, t.params.asset_decimals);
        let net = withhold_liquidation_fee(&t.env, &mut cache, 10_000_000, true, fee_raw);
        assert_eq!(net, 8_000_000);
        assert_eq!(cache.revenue(), expected_revenue);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #115)")]
fn test_withhold_liquidation_fee_rejects_fee_greater_than_gross() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.cache(100 * RAY, 50_000_000);
        let _ = withhold_liquidation_fee(&t.env, &mut cache, 1_000_000, true, 2_000_000);
    });
}
