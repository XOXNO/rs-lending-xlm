use super::*;
use crate::XoxnoOracle;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Env, Symbol};

#[test]
fn removing_an_unindexed_asset_or_feed_is_idempotent() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let oracle = env.register(XoxnoOracle, (admin, vec![&env, signer], 1u32, 300u32));
    let asset = ReflectorAsset::Other(Symbol::new(&env, "USD"));
    let feed = String::from_str(&env, "USD");

    env.as_contract(&oracle, || {
        asset_index_insert(&env, asset.clone());
        feed_index_insert(&env, feed.clone());
        assert_eq!((asset_count(&env), feed_count(&env)), (1, 1));

        asset_index_remove(&env, &asset);
        feed_index_remove(&env, &feed);
        assert_eq!((asset_count(&env), feed_count(&env)), (0, 0));

        asset_index_remove(&env, &asset);
        feed_index_remove(&env, &feed);
        assert_eq!((asset_count(&env), feed_count(&env)), (0, 0));
        assert!(!feed_index_contains(&env, &feed));
    });
}
