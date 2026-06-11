//! Test helpers for setting up RedStone mock adapters and market oracle configs.

use soroban_sdk::{Address, String};

use crate::core::types::LendingTest;
use crate::presets::DEFAULT_TOLERANCE;

/// Register a mock RedStone adapter and set initial prices for each feed.
///
/// `feeds` is a slice of `(feed_id, price_wad)` pairs.  Returns the adapter address.
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

/// Configure `symbol`'s market with a Reflector primary + RedStone anchor,
/// using the feed id equal to `symbol`.
///
/// Equivalent to `anchor_market_with_redstone_feed(t, adapter, symbol, symbol)`.
pub fn anchor_market_with_redstone(t: &LendingTest, adapter: &Address, symbol: &str) {
    anchor_market_with_redstone_feed(t, adapter, symbol, symbol);
}

/// Configure `symbol`'s market with a Reflector primary + RedStone anchor,
/// using an explicit `feed_id` (needed when two markets share one feed).
pub fn anchor_market_with_redstone_feed(
    t: &LendingTest,
    adapter: &Address,
    symbol: &str,
    feed_id: &str,
) {
    let asset = t.resolve_asset(symbol);
    let feed = String::from_str(&t.env, feed_id);
    let cfg = crate::oracle::config::reflector_primary_redstone_anchor_config(
        &t.mock_reflector,
        &asset,
        adapter,
        &feed,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.ctrl_client()
        .configure_market_oracle(&t.admin(), &asset, &cfg);
}

/// Return a client for the mock RedStone adapter at `adapter`.
pub fn redstone_counters<'a>(
    t: &'a LendingTest,
    adapter: &Address,
) -> crate::mock_redstone::MockRedStonePriceFeedClient<'a> {
    crate::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, adapter)
}
