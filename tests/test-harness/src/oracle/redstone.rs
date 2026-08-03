use soroban_sdk::{Address, String};

use crate::core::types::LendingTest;
use crate::presets::DEFAULT_TOLERANCE;

pub fn register_redstone_adapter(t: &LendingTest, feeds: &[(&str, i128)]) -> Address {
    let adapter = t
        .env
        .register(crate::mock_redstone::MockRedStonePriceFeed, ());
    let client = crate::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &adapter);
    for (feed, price_wad) in feeds {
        client.set_price(&String::from_str(&t.env, feed), price_wad);
    }
    adapter
}

pub fn anchor_market_with_redstone(t: &LendingTest, adapter: &Address, symbol: &str) {
    anchor_market_with_redstone_feed(t, adapter, symbol, symbol);
}

pub fn anchor_market_with_redstone_feed(
    t: &LendingTest,
    adapter: &Address,
    symbol: &str,
    feed_id: &str,
) {
    let asset = t.resolve_asset(symbol);
    let feed = String::from_str(&t.env, feed_id);
    let cfg = crate::oracle::config::reflector_primary_redstone_anchor_config(
        &t.env,
        &t.mock_reflector,
        &asset,
        adapter,
        &feed,
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&asset, &cfg);
}

pub fn redstone_counters<'a>(
    t: &'a LendingTest,
    adapter: &Address,
) -> crate::mock_redstone::MockRedStonePriceFeedClient<'a> {
    crate::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, adapter)
}
