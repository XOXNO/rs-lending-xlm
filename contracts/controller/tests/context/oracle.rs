//! First-pass killer for `Context::fetch_prices` → `()`.
extern crate std;

use crate::constants::WAD;
use crate::context::Context;
use crate::storage;
use crate::Controller;
use common::types::{PriceFeedRaw, PriceKey};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, Address, Env, Map, Vec};

#[contract]
struct StubAggregator;

#[contractimpl]
impl StubAggregator {
    pub fn prices(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw> {
        let mut out = Map::new(&env);
        for key in keys.iter() {
            out.set(
                key,
                PriceFeedRaw {
                    price_wad: WAD,
                    asset_decimals: 0,
                    timestamp: 0,
                },
            );
        }
        out
    }
}

#[test]
fn fetch_prices_populates_cache() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    let aggregator = env.register(StubAggregator, ());
    let asset = Address::generate(&env);
    env.as_contract(&id, || {
        storage::set_price_aggregator(&env, &aggregator);
        let mut cache = Context::new_view(&env);
        let mut assets = Vec::new(&env);
        assets.push_back(asset.clone());
        cache.fetch_prices(&assets);
        assert_eq!(cache.cached_price(&asset).price.raw(), WAD);
    });
}

#[test]
fn fetch_prices_skips_already_cached_assets() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    let asset = Address::generate(&env);
    env.as_contract(&id, || {
        let mut cache = Context::new_view(&env);
        let mut seeded = Map::new(&env);
        seeded.set(
            asset.clone(),
            PriceFeedRaw {
                price_wad: 2 * WAD,
                asset_decimals: 0,
                timestamp: 0,
            },
        );
        cache.set_prices(seeded);
        let mut assets = Vec::new(&env);
        assets.push_back(asset.clone());
        // No aggregator configured: a real fetch would panic.
        cache.fetch_prices(&assets);
        assert_eq!(cache.cached_price(&asset).price.raw(), 2 * WAD);
    });
}
