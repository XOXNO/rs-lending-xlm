//! Contract-surface tests: constructor ownership, owner-gated config
//! round-trip, and fail-closed pricing for unconfigured assets. Full price
//! resolution is covered by the moved engine unit tests and the controller
//! integration suite against the identical engine.

use common::types::{
    MarketOracleConfig, OraclePriceFluctuation, OracleSourceConfig, OracleSourceConfigOption,
    OracleStrategy, RedStoneSourceConfig,
};
use price_aggregator::{PriceAggregator, PriceAggregatorClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String, Vec};

fn redstone_single_config(env: &Env, feed: &Address) -> MarketOracleConfig {
    let price: i128 = 1_000_000_000_000_000_000;
    MarketOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        tolerance: OraclePriceFluctuation {
            upper_ratio_bps: 0,
            lower_ratio_bps: 0,
        },
        strategy: OracleStrategy::Single,
        primary: OracleSourceConfig::RedStone(RedStoneSourceConfig {
            contract: feed.clone(),
            feed_id: String::from_str(env, "BTC/USD"),
            decimals: 8,
            max_stale_seconds: 900,
        }),
        anchor: OracleSourceConfigOption::None,
        min_sanity_price_wad: price - price / 100,
        max_sanity_price_wad: price + price / 100,
    }
}

#[test]
fn set_market_oracle_config_roundtrips_through_storage() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let id = env.register(PriceAggregator, (owner,));
    let client = PriceAggregatorClient::new(&env, &id);

    let asset = Address::generate(&env);
    let feed = Address::generate(&env);
    let cfg = redstone_single_config(&env, &feed);

    client.set_market_oracle_config(&asset, &cfg);

    assert_eq!(client.get_asset_oracle(&asset), Some(cfg));
}

#[test]
#[should_panic(expected = "Error(Contract, #216)")]
fn prices_reverts_for_unconfigured_asset() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let id = env.register(PriceAggregator, (owner,));
    let client = PriceAggregatorClient::new(&env, &id);

    let asset = Address::generate(&env);
    // Fail-closed: an unconfigured asset reverts `OracleNotConfigured` (#216)
    // rather than returning a bad or missing price.
    client.prices(&Vec::from_array(&env, [asset]));
}
