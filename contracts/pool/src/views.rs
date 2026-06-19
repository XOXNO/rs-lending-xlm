use common::errors::GenericError;
use common::rates::{calculate_borrow_rate, calculate_deposit_rate};
use common::types::{MarketParamsRaw, PoolKey, PoolStateRaw, PoolSyncData};
use soroban_sdk::{panic_with_error, Address, Env};

use crate::cache::Cache;

// Raw keyed reads without TTL renewal.
pub fn load_state(env: &Env, asset: &Address) -> PoolStateRaw {
    env.storage()
        .persistent()
        .get(&PoolKey::State(asset.clone()))
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

pub fn load_params(env: &Env, asset: &Address) -> MarketParamsRaw {
    env.storage()
        .persistent()
        .get(&PoolKey::Params(asset.clone()))
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

// Bundles the asset's params and state for index simulation. Raw reads, no
// TTL renewal or interest accrual.
pub fn load_sync_data(env: &Env, asset: &Address) -> PoolSyncData {
    PoolSyncData {
        params: load_params(env, asset),
        state: load_state(env, asset),
    }
}

// Capital utilization ratio in RAY from the last persisted checkpoint.
// No interest accrual.
pub fn capital_utilisation(env: &Env, asset: &Address) -> i128 {
    Cache::load(env, asset).calculate_utilization().raw()
}

// Available reserves in asset decimals. Uses persisted `cash`, the liquidity
// source for borrows, withdrawals, and revenue claims.
// Direct token donations are excluded, matching `require_reserves` and
// `claim_revenue`. No TTL renewal or interest accrual.
pub fn reserves(env: &Env, asset: &Address) -> i128 {
    load_state(env, asset).cash
}

// Current deposit APR in RAY. Does not trigger interest accrual.
pub fn deposit_rate(env: &Env, asset: &Address) -> i128 {
    let cache = Cache::load(env, asset);
    let util = cache.calculate_utilization();
    let borrow = calculate_borrow_rate(env, util, &cache.params);
    calculate_deposit_rate(env, util, borrow, cache.params.reserve_factor).raw()
}

// Current borrow APR in RAY. Does not trigger interest accrual.
pub fn borrow_rate(env: &Env, asset: &Address) -> i128 {
    let cache = Cache::load(env, asset);
    calculate_borrow_rate(env, cache.calculate_utilization(), &cache.params).raw()
}

// Accrued protocol revenue in asset decimals. Does not trigger interest accrual.
pub fn protocol_revenue(env: &Env, asset: &Address) -> i128 {
    let cache = Cache::load(env, asset);
    cache.unscale_supply(cache.revenue)
}

// Total supplied in asset decimals. Does not trigger interest accrual.
pub fn supplied_amount(env: &Env, asset: &Address) -> i128 {
    let cache = Cache::load(env, asset);
    cache.unscale_supply(cache.supplied)
}

// Total borrowed in asset decimals. Does not trigger interest accrual.
pub fn borrowed_amount(env: &Env, asset: &Address) -> i128 {
    let cache = Cache::load(env, asset);
    cache.unscale_borrow(cache.borrowed)
}

// Milliseconds elapsed since last accrual. Does not trigger interest accrual.
pub fn delta_time(env: &Env, asset: &Address) -> u64 {
    let cache = Cache::load(env, asset);

    cache.current_timestamp.saturating_sub(cache.last_timestamp)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::test_support::init_ledger;
    use crate::{LiquidityPool, LiquidityPoolClient};
    use common::constants::RAY;
    use common::math::fp::Ray;
    use common::rates::{calculate_borrow_rate, calculate_deposit_rate};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{token, Address};

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
                max_borrow_rate_ray: 2 * RAY,
                base_borrow_rate_ray: RAY / 100,
                slope1_ray: RAY / 10,
                slope2_ray: RAY / 5,
                slope3_ray: RAY / 2,
                mid_utilization_ray: RAY / 2,
                optimal_utilization_ray: RAY * 8 / 10,
                max_utilization_ray: RAY * 95 / 100,
                reserve_factor_bps: 1_000,
                asset_id: asset.clone(),
                asset_decimals: 7,
            };
            let state = PoolStateRaw {
                supplied_ray: 10 * RAY,
                borrowed_ray: 5 * RAY,
                revenue_ray: 3 * RAY,
                borrow_index_ray: 3 * RAY,
                supply_index_ray: 2 * RAY,
                last_timestamp: 950_000,
                cash: 50_000_000,
            };
            let contract = env.register(LiquidityPool, (admin.clone(),));
            LiquidityPoolClient::new(&env, &contract).create_market(&params);

            env.as_contract(&contract, || {
                env.storage()
                    .persistent()
                    .set(&PoolKey::State(asset.clone()), &state);
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
            assert_eq!(load_params(&t.env, &t.asset).asset_id, t.asset);
            assert_eq!(load_state(&t.env, &t.asset).supplied_ray, 10 * RAY);
            // reserves() returns accounted `cash`; directly minted tokens are excluded.
            assert_eq!(reserves(&t.env, &t.asset), 50_000_000);
            // View amounts use asset decimals (7).
            // supplied: 10 scaled * 2.0 index = 20.0 -> 200_000_000 (7 dec).
            assert_eq!(supplied_amount(&t.env, &t.asset), 200_000_000);
            // borrowed: 5 scaled * 3.0 index = 15.0 -> 150_000_000 (7 dec).
            assert_eq!(borrowed_amount(&t.env, &t.asset), 150_000_000);
            // revenue: 3 scaled * 2.0 index = 6.0 -> 60_000_000 (7 dec).
            assert_eq!(protocol_revenue(&t.env, &t.asset), 60_000_000);
            // utilization stays in RAY (internal math).
            assert_eq!(capital_utilisation(&t.env, &t.asset), (15 * RAY) / 20);
            assert_eq!(delta_time(&t.env, &t.asset), 50_000);

            let util = Ray::from(capital_utilisation(&t.env, &t.asset));
            let params: common::types::MarketParams = (&t.params).into();
            let expected_borrow = calculate_borrow_rate(&t.env, util, &params);
            let expected_deposit =
                calculate_deposit_rate(&t.env, util, expected_borrow, params.reserve_factor);

            assert_eq!(borrow_rate(&t.env, &t.asset), expected_borrow.raw());
            assert_eq!(deposit_rate(&t.env, &t.asset), expected_deposit.raw());
        });
    }

    #[test]
    fn test_capital_utilisation_returns_zero_when_no_supply_exists() {
        let t = TestSetup::new();

        t.as_contract(|| {
            let zero_supply = PoolStateRaw {
                supplied_ray: 0,
                ..t.state.clone()
            };
            t.env
                .storage()
                .persistent()
                .set(&PoolKey::State(t.asset.clone()), &zero_supply);

            assert_eq!(capital_utilisation(&t.env, &t.asset), 0);
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
                .set(&PoolKey::State(t.asset.clone()), &future_state);

            assert_eq!(delta_time(&t.env, &t.asset), 0);
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
                .remove(&PoolKey::State(t.asset.clone()));
            let _ = load_state(&t.env, &t.asset);
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
                .remove(&PoolKey::Params(t.asset.clone()));
            let _ = load_params(&t.env, &t.asset);
        });
    }

    #[test]
    fn test_protocol_revenue_unscales_with_current_index() {
        let t = TestSetup::new();
        t.as_contract(|| {
            // revenue: 3 scaled * supply_index 2.0 = 6.0 asset units (7 decimals).
            assert_eq!(protocol_revenue(&t.env, &t.asset), 60_000_000);
        });
    }

    #[test]
    fn test_delta_time_matches_state_difference() {
        let t = TestSetup::new();
        t.as_contract(|| {
            // Fixture state sets last_timestamp 50k before current time.
            assert!(delta_time(&t.env, &t.asset) > 0);
        });
    }

    #[test]
    fn test_reserves_returns_accounted_cash_not_token_balance() {
        let t = TestSetup::new();
        // Fixture sets accounted `cash` (50_000_000) and token balance (12_345)
        // to different values; the token balance models an unsolicited donation.
        t.as_contract(|| {
            assert_eq!(reserves(&t.env, &t.asset), 50_000_000);
            assert_ne!(reserves(&t.env, &t.asset), 12_345);
        });

        // Another direct donation leaves reported reserves unchanged.
        token::StellarAssetClient::new(&t.env, &t.asset).mint(&t.contract, &1_000_000);
        t.as_contract(|| {
            assert_eq!(reserves(&t.env, &t.asset), 50_000_000);
        });
    }
}
