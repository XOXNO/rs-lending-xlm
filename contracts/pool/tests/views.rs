extern crate std;

use super::*;
use crate::storage::{load_state, read_params as load_params};
use crate::test_support::{hub, init_ledger};
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::{MILLISECONDS_PER_YEAR, RAY};
use common::math::fp_core::mul_div_half_up;
use common::types::{MarketParamsRaw, PoolKey, PoolStateRaw};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address};

struct TestSetup {
    env: Env,
    contract: Address,
    asset: Address,
    state: PoolStateRaw,
}

impl TestSetup {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        init_ledger(&env);

        let admin = Address::generate(&env);
        let asset = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address()
            .clone();
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
        let state = PoolStateRaw {
            supplied: 10 * RAY,
            borrowed: 5 * RAY,
            revenue: 3 * RAY,
            borrow_index: 3 * RAY,
            supply_index: 2 * RAY,
            last_timestamp: 950_000,
            cash: 50_000_000,
        };
        let contract = env.register(LiquidityPool, (admin.clone(),));
        LiquidityPoolClient::new(&env, &contract).create_market(&0u32, &params);

        env.as_contract(&contract, || {
            env.storage()
                .persistent()
                .set(&PoolKey::State(hub(&asset)), &state);
        });

        let token_admin = token::StellarAssetClient::new(&env, &asset);
        token_admin.mint(&contract, &12_345);

        Self {
            env,
            contract,
            asset,
            state,
        }
    }

    fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
        self.env.as_contract(&self.contract, f)
    }
}

/// Per-millisecond form of a 200% APR — larger than any accrual rate the curve
/// can emit. View getters must return annual RAY, so a miswired per-ms value
/// cannot pass this bound.
fn assert_annual_apr(rate: i128, label: &str) {
    let max_per_ms = (2 * RAY) / (MILLISECONDS_PER_YEAR as i128);
    assert!(
        rate > max_per_ms,
        "{label} view must be annual RAY APR, got {rate} (200% per-ms is {max_per_ms})"
    );
}

#[test]
fn test_views_load_and_compute_expected_values() {
    let t = TestSetup::new();

    t.as_contract(|| {
        assert_eq!(load_params(&t.env, &hub(&t.asset)).asset_id, t.asset);
        assert_eq!(load_state(&t.env, &hub(&t.asset)).supplied, 10 * RAY);

        assert_eq!(reserves(&t.env, &hub(&t.asset)), 50_000_000);

        assert_eq!(supplied_amount(&t.env, &hub(&t.asset)), 200_000_000);

        assert_eq!(borrowed_amount(&t.env, &hub(&t.asset)), 150_000_000);

        assert_eq!(protocol_revenue(&t.env, &hub(&t.asset)), 60_000_000);

        assert_eq!(utilization(&t.env, &hub(&t.asset)), (15 * RAY) / 20);
        assert_eq!(delta_time(&t.env, &hub(&t.asset)), 50_000);

        // Value utilization is 75% (15/20 after indexes). Share ratio is 50%.
        // Region 2: 1% + 10% + (25/30)*20% = 83/300 APR, half-up.
        // Deposit APR = 75% * (83/300) * 90% = 18.675%.
        let expected_borrow_apr = mul_div_half_up(&t.env, 83, RAY, 300);
        let expected_deposit_apr = RAY * 18_675 / 100_000;
        assert_eq!(borrow_rate(&t.env, &hub(&t.asset)), expected_borrow_apr);
        assert_eq!(deposit_rate(&t.env, &hub(&t.asset)), expected_deposit_apr);
        assert_annual_apr(expected_borrow_apr, "borrow");
        assert_annual_apr(expected_deposit_apr, "deposit");
    });

    let client = LiquidityPoolClient::new(&t.env, &t.contract);
    let key = hub(&t.asset);
    let borrow = client.get_borrow_rate(&key);
    let deposit = client.get_deposit_rate(&key);
    assert_eq!(client.get_utilisation(&key), (15 * RAY) / 20);
    assert_eq!(borrow, mul_div_half_up(&t.env, 83, RAY, 300));
    assert_eq!(deposit, RAY * 18_675 / 100_000);
    assert_annual_apr(borrow, "borrow");
    assert_annual_apr(deposit, "deposit");
}

#[test]
fn test_utilization_returns_zero_when_no_supply_exists() {
    let t = TestSetup::new();

    t.as_contract(|| {
        let zero_supply = PoolStateRaw {
            supplied: 0,
            ..t.state.clone()
        };
        t.env
            .storage()
            .persistent()
            .set(&PoolKey::State(hub(&t.asset)), &zero_supply);

        assert_eq!(utilization(&t.env, &hub(&t.asset)), 0);
    });
}

#[test]
fn test_delta_time_saturates_when_last_timestamp_is_in_future() {
    let t = TestSetup::new();

    t.as_contract(|| {
        let future_state = PoolStateRaw {
            last_timestamp: 1_100_000,
            ..t.state.clone()
        };
        t.env
            .storage()
            .persistent()
            .set(&PoolKey::State(hub(&t.asset)), &future_state);

        assert_eq!(delta_time(&t.env, &hub(&t.asset)), 0);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn test_load_state_panics_when_pool_is_not_initialized() {
    let t = TestSetup::new();
    t.as_contract(|| {
        t.env
            .storage()
            .persistent()
            .remove(&PoolKey::State(hub(&t.asset)));
        let _ = load_state(&t.env, &hub(&t.asset));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn test_load_params_panics_when_pool_is_not_initialized() {
    let t = TestSetup::new();
    t.as_contract(|| {
        t.env
            .storage()
            .persistent()
            .remove(&PoolKey::Params(hub(&t.asset)));
        let _ = load_params(&t.env, &hub(&t.asset));
    });
}

#[test]
fn test_protocol_revenue_unscales_with_current_index() {
    let t = TestSetup::new();
    t.as_contract(|| {
        assert_eq!(protocol_revenue(&t.env, &hub(&t.asset)), 60_000_000);
    });
}

#[test]
fn test_delta_time_matches_state_difference() {
    let t = TestSetup::new();
    t.as_contract(|| {
        assert_eq!(delta_time(&t.env, &hub(&t.asset)), 50_000);
    });
}

#[test]
fn test_reserves_returns_accounted_cash_not_token_balance() {
    let t = TestSetup::new();

    t.as_contract(|| {
        assert_eq!(reserves(&t.env, &hub(&t.asset)), 50_000_000);
        assert_ne!(reserves(&t.env, &hub(&t.asset)), 12_345);
    });

    token::StellarAssetClient::new(&t.env, &t.asset).mint(&t.contract, &1_000_000);
    t.as_contract(|| {
        assert_eq!(reserves(&t.env, &hub(&t.asset)), 50_000_000);
    });
}
