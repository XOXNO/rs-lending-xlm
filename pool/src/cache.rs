use common::errors::GenericError;
use common::fp::Ray;
use common::types::{MarketParams, PoolKey, PoolState};
use soroban_sdk::{panic_with_error, Env};

pub struct Cache {
    pub env: Env,
    pub supplied: Ray,
    pub borrowed: Ray,
    pub revenue: Ray,
    pub borrow_index: Ray,
    pub supply_index: Ray,
    pub last_timestamp: u64,
    pub current_timestamp: u64,
    pub params: MarketParams,
}

impl Cache {
    /// Loads pool params and state from instance storage.
    /// Initializes all indexes to 1 and balances to zero when no state entry exists yet.
    pub fn load(env: &Env) -> Self {
        let params: MarketParams = env
            .storage()
            .instance()
            .get(&PoolKey::Params)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));

        let state: Option<PoolState> = env.storage().instance().get(&PoolKey::State);
        let time_ms = env.ledger().timestamp() * 1000;
        match state {
            Some(s) => Cache {
                env: env.clone(),
                supplied: Ray::from_raw(s.supplied_ray),
                borrowed: Ray::from_raw(s.borrowed_ray),
                revenue: Ray::from_raw(s.revenue_ray),
                borrow_index: Ray::from_raw(s.borrow_index_ray),
                supply_index: Ray::from_raw(s.supply_index_ray),
                last_timestamp: s.last_timestamp,
                current_timestamp: time_ms,
                params,
            },
            None => Cache {
                env: env.clone(),
                supplied: Ray::ZERO,
                borrowed: Ray::ZERO,
                revenue: Ray::ZERO,
                borrow_index: Ray::ONE,
                supply_index: Ray::ONE,
                last_timestamp: 0,
                current_timestamp: time_ms,
                params,
            },
        }
    }

    /// Persists the current cache values to instance storage.
    pub fn save(&self) {
        let state = PoolState {
            supplied_ray: self.supplied.raw(),
            borrowed_ray: self.borrowed.raw(),
            revenue_ray: self.revenue.raw(),
            borrow_index_ray: self.borrow_index.raw(),
            supply_index_ray: self.supply_index.raw(),
            last_timestamp: self.last_timestamp,
        };

        self.env.storage().instance().set(&PoolKey::State, &state);
    }

    // -----------------------------------------------------------------------
    // Helper methods
    // -----------------------------------------------------------------------

    /// Returns utilization as `(borrowed × borrow_index) / (supplied × supply_index)` in RAY.
    /// Returns 0 when total supply or the product is zero.
    pub fn calculate_utilization(&self) -> i128 {
        if self.supplied == Ray::ZERO {
            return 0;
        }
        let total_borrowed = self.borrowed.mul(&self.env, self.borrow_index);
        let total_supplied = self.supplied.mul(&self.env, self.supply_index);
        if total_supplied == Ray::ZERO {
            return 0;
        }
        total_borrowed.div(&self.env, total_supplied).raw()
    }

    /// Returns `true` when the pool's on-chain token balance covers `amount`.
    pub fn has_reserves(&self, amount: i128) -> bool {
        let reserves = self.get_reserves_for(&self.params.asset_id);
        reserves >= amount
    }

    /// Queries the pool contract's current on-chain balance of `asset`.
    pub fn get_reserves_for(&self, asset: &soroban_sdk::Address) -> i128 {
        let token = soroban_sdk::token::Client::new(&self.env, asset);
        token.balance(&self.env.current_contract_address())
    }

    /// Converts an asset-decimal amount to a RAY-scaled value: `rescale(amount, dec, 27) / index`.
    pub fn calculate_scaled_supply(&self, amount: i128) -> Ray {
        let amount_ray = Ray::from_asset(amount, self.params.asset_decimals);
        amount_ray.div(&self.env, self.supply_index)
    }

    /// Converts an asset-decimal amount to a RAY-scaled value: `rescale(amount, dec, 27) / index`.
    pub fn calculate_scaled_borrow(&self, amount: i128) -> Ray {
        let amount_ray = Ray::from_asset(amount, self.params.asset_decimals);
        amount_ray.div(&self.env, self.borrow_index)
    }

    /// Recovers the actual amount in RAY precision: `scaled * index` (stays in RAY).
    pub fn calculate_original_borrow_ray(&self, scaled: Ray) -> Ray {
        scaled.mul(&self.env, self.borrow_index)
    }

    /// Recovers the actual amount in asset decimals for token transfers.
    pub fn calculate_original_supply(&self, scaled: Ray) -> i128 {
        scaled
            .mul(&self.env, self.supply_index)
            .to_asset(self.params.asset_decimals)
    }

    /// Recovers the actual amount in asset decimals for token transfers.
    pub fn calculate_original_borrow(&self, scaled: Ray) -> i128 {
        scaled
            .mul(&self.env, self.borrow_index)
            .to_asset(self.params.asset_decimals)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::constants::RAY;
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::Address;

    struct TestSetup {
        env: Env,
        contract: Address,
        params: MarketParams,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set(LedgerInfo {
                timestamp: 1_000,
                protocol_version: 26,
                sequence_number: 100,
                network_id: Default::default(),
                base_reserve: 10,
                min_temp_entry_ttl: 10,
                min_persistent_entry_ttl: 10,
                max_entry_ttl: 3_110_400,
            });

            let admin = Address::generate(&env);
            let params = MarketParams {
                max_borrow_rate_ray: 5 * RAY,
                base_borrow_rate_ray: RAY / 100,
                slope1_ray: RAY / 10,
                slope2_ray: RAY / 5,
                slope3_ray: RAY / 2,
                mid_utilization_ray: RAY / 2,
                optimal_utilization_ray: RAY * 8 / 10,
                reserve_factor_bps: 1_000,
                asset_id: Address::generate(&env),
                asset_decimals: 7,
            };
            let contract = env.register(crate::LiquidityPool, (admin.clone(), params.clone()));

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

    #[test]
    fn test_load_uses_neutral_defaults_when_state_is_missing() {
        let t = TestSetup::new();

        t.as_contract(|| {
            t.env.storage().instance().remove(&PoolKey::State);
            let cache = Cache::load(&t.env);

            assert_eq!(cache.supplied, Ray::ZERO);
            assert_eq!(cache.borrowed, Ray::ZERO);
            assert_eq!(cache.revenue, Ray::ZERO);
            assert_eq!(cache.borrow_index, Ray::ONE);
            assert_eq!(cache.supply_index, Ray::ONE);
            assert_eq!(cache.last_timestamp, 0);
            assert_eq!(cache.current_timestamp, 1_000_000);
            assert_eq!(cache.params.asset_id, t.params.asset_id);
        });
    }

    #[test]
    fn test_calculate_utilization_returns_zero_when_supply_index_zeroes_total_supply() {
        let t = TestSetup::new();

        t.as_contract(|| {
            let cache = Cache {
                env: t.env.clone(),
                supplied: Ray::from_raw(10 * RAY),
                borrowed: Ray::from_raw(5 * RAY),
                revenue: Ray::ZERO,
                borrow_index: Ray::from_raw(2 * RAY),
                supply_index: Ray::ZERO,
                last_timestamp: 0,
                current_timestamp: 1_000_000,
                params: t.params.clone(),
            };

            assert_eq!(cache.calculate_utilization(), 0);
        });
    }
}
