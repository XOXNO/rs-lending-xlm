extern crate std;

use super::*;
use crate::test_support::init_ledger;
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::RAY;
use common::math::fp::Ray;
use common::rates::{calculate_borrow_rate, calculate_deposit_rate};
use common::types::MarketParams;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address};

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
    asset: Address,
    params: MarketParamsRaw,
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
            params,
            state,
        }
    }

    fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
        self.env.as_contract(&self.contract, f)
    }
}

#[test]
fn test_views_load_and_compute_expected_values() {
    let t = TestSetup::new();

    t.as_contract(|| {
        assert_eq!(load_params(&t.env, &hub(&t.asset)).asset_id, t.asset);
        assert_eq!(load_state(&t.env, &hub(&t.asset)).supplied, 10 * RAY);
        // reserves() returns accounted `cash`; directly minted tokens are excluded.
        assert_eq!(reserves(&t.env, &hub(&t.asset)), 50_000_000);
        // View amounts use asset decimals (7).
        // supplied: 10 scaled * 2.0 index = 20.0 -> 200_000_000 (7 dec).
        assert_eq!(supplied_amount(&t.env, &hub(&t.asset)), 200_000_000);
        // borrowed: 5 scaled * 3.0 index = 15.0 -> 150_000_000 (7 dec).
        assert_eq!(borrowed_amount(&t.env, &hub(&t.asset)), 150_000_000);
        // revenue: 3 scaled * 2.0 index = 6.0 -> 60_000_000 (7 dec).
        assert_eq!(protocol_revenue(&t.env, &hub(&t.asset)), 60_000_000);
        // utilization stays in RAY (internal math).
        assert_eq!(capital_utilisation(&t.env, &hub(&t.asset)), (15 * RAY) / 20);
        assert_eq!(delta_time(&t.env, &hub(&t.asset)), 50_000);

        let util = Ray::from(capital_utilisation(&t.env, &hub(&t.asset)));
        let params: MarketParams = (&t.params).into();
        let expected_borrow = calculate_borrow_rate(&t.env, util, &params);
        let expected_deposit =
            calculate_deposit_rate(&t.env, util, expected_borrow, params.reserve_factor);

        assert_eq!(borrow_rate(&t.env, &hub(&t.asset)), expected_borrow.raw());
        assert_eq!(deposit_rate(&t.env, &hub(&t.asset)), expected_deposit.raw());
    });
}

#[test]
fn test_capital_utilisation_returns_zero_when_no_supply_exists() {
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

        assert_eq!(capital_utilisation(&t.env, &hub(&t.asset)), 0);
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
        // revenue: 3 scaled * supply_index 2.0 = 6.0 asset units (7 decimals).
        assert_eq!(protocol_revenue(&t.env, &hub(&t.asset)), 60_000_000);
    });
}

#[test]
fn test_delta_time_matches_state_difference() {
    let t = TestSetup::new();
    t.as_contract(|| {
        // Fixture state sets last_timestamp 50k before current time.
        assert!(delta_time(&t.env, &hub(&t.asset)) > 0);
    });
}

#[test]
fn test_reserves_returns_accounted_cash_not_token_balance() {
    let t = TestSetup::new();
    // Fixture sets accounted `cash` (50_000_000) and token balance (12_345)
    // to different values; the token balance models an unsolicited donation.
    t.as_contract(|| {
        assert_eq!(reserves(&t.env, &hub(&t.asset)), 50_000_000);
        assert_ne!(reserves(&t.env, &hub(&t.asset)), 12_345);
    });

    // Another direct donation leaves reported reserves unchanged.
    token::StellarAssetClient::new(&t.env, &t.asset).mint(&t.contract, &1_000_000);
    t.as_contract(|| {
        assert_eq!(reserves(&t.env, &hub(&t.asset)), 50_000_000);
    });
}
