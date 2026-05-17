use common::errors::GenericError;
use common::rates::{calculate_borrow_rate, calculate_deposit_rate};
use common::types::{MarketParams, PoolKey, PoolState};
use soroban_sdk::{panic_with_error, Env};

use crate::cache::Cache;

/// Loads pool state; panics `PoolNotInitialized` if missing.
pub fn load_state(env: &Env) -> PoolState {
    env.storage()
        .instance()
        .get(&PoolKey::State)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Loads market params; panics `PoolNotInitialized` if missing.
pub fn load_params(env: &Env) -> MarketParams {
    env.storage()
        .instance()
        .get(&PoolKey::Params)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Capital utilisation ratio in RAY.
pub fn capital_utilisation(env: &Env) -> i128 {
    Cache::load(env).calculate_utilization().raw()
}

/// Pool's live token reserves in asset decimals.
pub fn reserves(env: &Env) -> i128 {
    let params = load_params(env);
    let token = soroban_sdk::token::Client::new(env, &params.asset_id);
    token.balance(&env.current_contract_address())
}

/// Current deposit APR in RAY.
pub fn deposit_rate(env: &Env) -> i128 {
    let cache = Cache::load(env);
    let util = cache.calculate_utilization();
    let borrow = calculate_borrow_rate(env, util, &cache.params);
    calculate_deposit_rate(env, util, borrow, cache.params.reserve_factor_bps).raw()
}

/// Current borrow APR in RAY.
pub fn borrow_rate(env: &Env) -> i128 {
    let cache = Cache::load(env);
    calculate_borrow_rate(env, cache.calculate_utilization(), &cache.params).raw()
}

/// Accrued protocol revenue in asset decimals.
pub fn protocol_revenue(env: &Env) -> i128 {
    let c = Cache::load(env);
    c.calculate_original_supply(c.revenue)
}

/// Total supplied in asset decimals.
pub fn supplied_amount(env: &Env) -> i128 {
    let c = Cache::load(env);
    c.calculate_original_supply(c.supplied)
}

/// Total borrowed in asset decimals.
pub fn borrowed_amount(env: &Env) -> i128 {
    let c = Cache::load(env);
    c.calculate_original_borrow(c.borrowed)
}

/// Milliseconds elapsed since the last interest accrual.
pub fn delta_time(env: &Env) -> u64 {
    let state = load_state(env);
    let current_ms = env.ledger().timestamp() * 1000;
    current_ms.saturating_sub(state.last_timestamp)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::test_support::init_ledger;
    use common::constants::RAY;
    use common::fp::Ray;
    use common::rates::{calculate_borrow_rate, calculate_deposit_rate};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{token, Address};

    struct TestSetup {
        env: Env,
        contract: Address,
        asset: Address,
        params: MarketParams,
        state: PoolState,
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
            let params = MarketParams {
                max_borrow_rate_ray: 2 * RAY,
                base_borrow_rate_ray: RAY / 100,
                slope1_ray: RAY / 10,
                slope2_ray: RAY / 5,
                slope3_ray: RAY / 2,
                mid_utilization_ray: RAY / 2,
                optimal_utilization_ray: RAY * 8 / 10,
                reserve_factor_bps: 1_000,
                asset_id: asset.clone(),
                asset_decimals: 7,
            };
            let state = PoolState {
                supplied_ray: 10 * RAY,
                borrowed_ray: 5 * RAY,
                revenue_ray: 3 * RAY,
                borrow_index_ray: 3 * RAY,
                supply_index_ray: 2 * RAY,
                last_timestamp: 950_000,
            };
            let contract = env.register(crate::LiquidityPool, (admin.clone(), params.clone()));

            env.as_contract(&contract, || {
                env.storage().instance().set(&PoolKey::State, &state);
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
            assert_eq!(load_params(&t.env).asset_id, t.asset);
            assert_eq!(load_state(&t.env).supplied_ray, 10 * RAY);
            assert_eq!(reserves(&t.env), 12_345);
            // View amounts are returned in asset decimals (7).
            // supplied: 10 scaled * 2.0 index = 20.0 -> 200_000_000 (7 dec).
            assert_eq!(supplied_amount(&t.env), 200_000_000);
            // borrowed: 5 scaled * 3.0 index = 15.0 -> 150_000_000 (7 dec).
            assert_eq!(borrowed_amount(&t.env), 150_000_000);
            // revenue: 3 scaled * 2.0 index = 6.0 -> 60_000_000 (7 dec).
            assert_eq!(protocol_revenue(&t.env), 60_000_000);
            // utilization stays in RAY (internal math).
            assert_eq!(capital_utilisation(&t.env), (15 * RAY) / 20);
            assert_eq!(delta_time(&t.env), 50_000);

            let util = Ray::from_raw(capital_utilisation(&t.env));
            let expected_borrow = calculate_borrow_rate(&t.env, util, &t.params);
            let expected_deposit =
                calculate_deposit_rate(&t.env, util, expected_borrow, t.params.reserve_factor_bps);

            assert_eq!(borrow_rate(&t.env), expected_borrow.raw());
            assert_eq!(deposit_rate(&t.env), expected_deposit.raw());
        });
    }

    #[test]
    fn test_capital_utilisation_returns_zero_when_no_supply_exists() {
        let t = TestSetup::new();

        t.as_contract(|| {
            let zero_supply = PoolState {
                supplied_ray: 0,
                ..t.state.clone()
            };
            t.env
                .storage()
                .instance()
                .set(&PoolKey::State, &zero_supply);

            assert_eq!(capital_utilisation(&t.env), 0);
        });
    }

    #[test]
    fn test_delta_time_saturates_when_last_timestamp_is_in_future() {
        let t = TestSetup::new();

        t.as_contract(|| {
            let future_state = PoolState {
                last_timestamp: 1_100_000,
                ..t.state.clone()
            };
            t.env
                .storage()
                .instance()
                .set(&PoolKey::State, &future_state);

            assert_eq!(delta_time(&t.env), 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #30)")]
    fn test_load_state_panics_when_pool_is_not_initialized() {
        let t = TestSetup::new();
        t.as_contract(|| {
            t.env.storage().instance().remove(&PoolKey::State);
            let _ = load_state(&t.env);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #30)")]
    fn test_load_params_panics_when_pool_is_not_initialized() {
        let t = TestSetup::new();
        t.as_contract(|| {
            t.env.storage().instance().remove(&PoolKey::Params);
            let _ = load_params(&t.env);
        });
    }
}
