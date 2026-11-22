//! Batched market indexes must populate the cache before later single-key reads.

use crate::context::Context;
use crate::storage;
use crate::Controller;
use common::constants::RAY;
use common::types::{HubAssetKey, MarketIndexRaw};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, symbol_short, vec, Address, Env, Vec};

#[contract]
struct IndexPool;

#[contractimpl]
impl IndexPool {
    pub fn get_bulk_indexes(env: Env, keys: Vec<HubAssetKey>) -> Vec<MarketIndexRaw> {
        let calls: u32 = env
            .storage()
            .instance()
            .get(&symbol_short!("calls"))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&symbol_short!("calls"), &(calls + 1));

        let mut indexes = Vec::new(&env);
        for key in keys.iter() {
            indexes.push_back(MarketIndexRaw {
                borrow_index: RAY + i128::from(key.hub_id),
                supply_index: RAY,
            });
        }
        indexes
    }

    pub fn calls(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("calls"))
            .unwrap_or(0)
    }
}

fn key(env: &Env, hub_id: u32) -> HubAssetKey {
    HubAssetKey {
        hub_id,
        asset: Address::generate(env),
    }
}

#[test]
fn batched_indexes_are_fetched_once_and_reused() {
    let env = Env::default();
    let controller = env.register(Controller, (Address::generate(&env),));
    let pool = env.register(IndexPool, ());
    let pool_client = IndexPoolClient::new(&env, &pool);
    let first = key(&env, 1);
    let second = key(&env, 2);
    let third = key(&env, 3);

    env.as_contract(&controller, || {
        storage::set_pool(&env, &pool);
        let mut cache = Context::new_view(&env);

        cache.fetch_market_indexes(&vec![&env, first.clone(), second.clone()]);
        assert_eq!(pool_client.calls(), 1);
        assert_eq!(cache.market_indexes.len(), 2);
        assert_eq!(
            cache
                .market_indexes
                .get_unchecked(first.clone())
                .borrow_index,
            RAY + 1
        );
        assert_eq!(
            cache
                .market_indexes
                .get_unchecked(second.clone())
                .borrow_index,
            RAY + 2
        );

        cache.fetch_market_indexes(&vec![&env, first.clone(), second]);
        assert_eq!(pool_client.calls(), 1, "cached indexes need no pool call");

        cache.fetch_market_indexes(&vec![&env, first, third.clone()]);
        assert_eq!(pool_client.calls(), 2, "only the new key is fetched");
        assert_eq!(
            cache.market_indexes.get_unchecked(third).borrow_index,
            RAY + 3
        );
    });
}
