use common::events::{emit_update_debt_ceiling, UpdateDebtCeilingEvent};
use common::types::{EModeAssetConfig, MarketConfig, MarketIndex, PriceFeed};
use soroban_sdk::{Address, Env, Map};

use crate::storage;

pub struct ControllerCache {
    env: Env,

    // --- Cached maps (get-or-fetch) ---
    pub prices_cache: Map<Address, PriceFeed>,
    pub market_configs: Map<Address, MarketConfig>,
    pub market_indexes: Map<Address, MarketIndex>,

    // --- E-mode asset membership read cache ---
    emode_assets: Map<(u32, Address), Option<EModeAssetConfig>>,

    // --- Isolated-debt write accumulator ---
    isolated_debts: Map<Address, i128>,

    pub current_timestamp_ms: u64,
    pub allow_unsafe_price: bool,
    pub allow_disabled_market_price: bool,
    pub simulate: bool,
}

impl ControllerCache {
    pub fn new(env: &Env, allow_unsafe_price: bool) -> Self {
        Self::build(env, allow_unsafe_price, false, true)
    }

    pub fn new_with_disabled_market_price(env: &Env, allow_unsafe_price: bool) -> Self {
        Self::build(env, allow_unsafe_price, true, true)
    }

    pub fn new_view(env: &Env) -> Self {
        Self::build(env, true, true, false)
    }

    pub(crate) fn build(
        env: &Env,
        allow_unsafe_price: bool,
        allow_disabled_market_price: bool,
        bump_ttl: bool,
    ) -> Self {
        let current_timestamp_ms = env.ledger().timestamp() * 1000;

        ControllerCache {
            env: env.clone(),
            prices_cache: Map::new(env),
            market_configs: Map::new(env),
            market_indexes: Map::new(env),
            emode_assets: Map::new(env),
            isolated_debts: Map::new(env),
            current_timestamp_ms,
            allow_unsafe_price,
            allow_disabled_market_price,
            simulate: !bump_ttl,
        }
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    // -------------------------------------------------------------------
    // Prices (single cache -- oracle module resolves tolerance internally)
    // -------------------------------------------------------------------

    pub fn try_get_price(&self, asset: &Address) -> Option<PriceFeed> {
        self.prices_cache.get(asset.clone())
    }

    pub fn set_price(&mut self, asset: &Address, feed: &PriceFeed) {
        self.prices_cache.set(asset.clone(), feed.clone());
    }

    pub fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        crate::oracle::token_price(self, asset)
    }

    pub fn clean_prices_cache(&mut self) {
        self.prices_cache = Map::new(&self.env);
    }

    // -------------------------------------------------------------------
    // Market config (consolidated) -- bumps shared TTL on first load
    // -------------------------------------------------------------------

    pub fn cached_market_config(&mut self, asset: &Address) -> MarketConfig {
        if let Some(config) = self.market_configs.get(asset.clone()) {
            return config;
        }
        let config = storage::get_market_config(&self.env, asset);
        self.market_configs.set(asset.clone(), config.clone());
        config
    }

    // -------------------------------------------------------------------
    // Convenience accessors -- delegate to cached_market_config so callers
    // that need only one field stay unchanged.
    // -------------------------------------------------------------------

    pub fn cached_asset_config(&mut self, asset: &Address) -> common::types::AssetConfig {
        self.cached_market_config(asset).asset_config
    }

    pub fn cached_pool_address(&mut self, asset: &Address) -> Address {
        self.cached_market_config(asset).pool_address
    }

    // -------------------------------------------------------------------
    // Market indexes
    // -------------------------------------------------------------------

    pub fn cached_market_index(&mut self, asset: &Address) -> MarketIndex {
        if let Some(index) = self.market_indexes.get(asset.clone()) {
            return index;
        }
        let simulate = self.simulate;
        let index = crate::oracle::update_asset_index(self, asset, simulate);
        self.market_indexes.set(asset.clone(), index.clone());
        index
    }

    pub fn cached_market_index_readonly(&mut self, asset: &Address) -> MarketIndex {
        if let Some(index) = self.market_indexes.get(asset.clone()) {
            return index;
        }
        let index = crate::oracle::update_asset_index(self, asset, true);
        self.market_indexes.set(asset.clone(), index.clone());
        index
    }

    // -------------------------------------------------------------------
    // E-mode asset membership read cache
    // -------------------------------------------------------------------

    pub fn cached_emode_asset(
        &mut self,
        category_id: u32,
        asset: &Address,
    ) -> Option<EModeAssetConfig> {
        if category_id == 0 {
            return None;
        }
        let key = (category_id, asset.clone());
        if let Some(cached) = self.emode_assets.get(key.clone()) {
            return cached;
        }
        let value = storage::get_emode_asset(&self.env, category_id, asset);
        self.emode_assets.set(key, value.clone());
        value
    }

    // -------------------------------------------------------------------
    // Isolated-debt write accumulator
    // -------------------------------------------------------------------

    pub fn get_isolated_debt(&mut self, asset: &Address) -> i128 {
        if let Some(v) = self.isolated_debts.get(asset.clone()) {
            return v;
        }
        let v = storage::get_isolated_debt(&self.env, asset);
        // Cache the value so future reads in the same transaction skip
        // another storage access.
        self.isolated_debts.set(asset.clone(), v);
        v
    }

    pub fn set_isolated_debt(&mut self, asset: &Address, value: i128) {
        self.isolated_debts.set(asset.clone(), value);
    }

    pub fn flush_isolated_debts(&self) {
        for asset in self.isolated_debts.keys() {
            let value = self.isolated_debts.get(asset.clone()).unwrap();
            storage::set_isolated_debt(&self.env, &asset, value);
            emit_update_debt_ceiling(
                &self.env,
                UpdateDebtCeilingEvent {
                    asset,
                    total_debt_usd_wad: value,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::types::EModeAssetConfig;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    struct TestSetup {
        env: Env,
        controller: Address,
        asset: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let controller = env.register(crate::Controller, (admin,));
            let asset = Address::generate(&env);

            Self {
                env,
                controller,
                asset,
            }
        }

        fn as_controller<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.controller, f)
        }
    }

    #[test]
    fn test_cached_emode_asset_returns_cached_value_on_second_lookup() {
        let t = TestSetup::new();

        t.as_controller(|| {
            let expected = EModeAssetConfig {
                is_collateralizable: true,
                is_borrowable: false,
            };
            storage::set_emode_asset(&t.env, 7, &t.asset, &expected);

            let mut cache = ControllerCache::new_view(&t.env);
            let first = cache.cached_emode_asset(7, &t.asset).unwrap();
            assert!(first.is_collateralizable);
            assert!(!first.is_borrowable);

            storage::remove_emode_asset(&t.env, 7, &t.asset);
            let second = cache.cached_emode_asset(7, &t.asset).unwrap();
            assert!(second.is_collateralizable);
            assert!(!second.is_borrowable);
        });
    }
}
