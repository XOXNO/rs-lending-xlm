//! One oracle and market snapshot per rule.
//!
//! `price_feed_summary` and `get_sync_data_summary` draw a fresh
//! nondeterministic value on every call. `Cache` memoises within one instance,
//! but a verb creates several caches and a rule usually creates one of its
//! own, so without this file two reads of the same asset in one rule return
//! two unrelated prices, and the market index a rule values a position with is
//! unrelated to the one the gate inside the call used.
//!
//! These maps record the first draw for a key and replay it for the rest of
//! the rule, which is the model of INV-ORACLE-03, "one transaction sees one
//! snapshot": within a rule one asset has one price and one market has one
//! parameter and index pair. `market_index` is derived from the same sync
//! draw the pool's sync view returns, so `Cache::cached_market_index` and
//! `Cache::cached_pool_sync_data` can no longer disagree about one market.
//!
//! The memo only removes behaviours, so it cannot make a universal rule pass
//! that would otherwise fail on a genuinely reachable state; the states it
//! removes are the ones production cannot produce.

use crate::spec::summaries::pool::get_sync_data_summary;
use crate::spec::summaries::price_feed_summary;
use crate::types::{HubAssetKey, MarketIndexRaw, PoolSyncData, PriceFeedRaw};
use soroban_sdk::{Address, Env, Map};

static mut GHOST_PRICES: Option<Map<Address, PriceFeedRaw>> = None;
static mut GHOST_SYNC: Option<Map<HubAssetKey, PoolSyncData>> = None;

/// Drops both snapshots. Rules start with them empty; this exists so a rule
/// that wants two independent draws can say so explicitly.
pub fn reset() {
    unsafe {
        GHOST_PRICES = None;
        GHOST_SYNC = None;
    }
}

/// The price this rule has already seen for `asset`, or a fresh draw recorded
/// for every later read.
pub(crate) fn price(env: &Env, asset: &Address) -> PriceFeedRaw {
    unsafe {
        let mut prices = match &*core::ptr::addr_of!(GHOST_PRICES) {
            Some(prices) => prices.clone(),
            None => Map::new(env),
        };
        if let Some(feed) = prices.get(asset.clone()) {
            return feed;
        }
        let feed = price_feed_summary(env, asset);
        prices.set(asset.clone(), feed.clone());
        GHOST_PRICES = Some(prices);
        feed
    }
}

/// The pool sync data this rule has already seen for `hub_asset`, or a fresh
/// draw recorded for every later read.
pub(crate) fn sync_data(env: &Env, hub_asset: &HubAssetKey) -> PoolSyncData {
    unsafe {
        let mut markets = match &*core::ptr::addr_of!(GHOST_SYNC) {
            Some(markets) => markets.clone(),
            None => Map::new(env),
        };
        if let Some(sync) = markets.get(hub_asset.clone()) {
            return sync;
        }
        let sync = get_sync_data_summary(env, &hub_asset.asset);
        markets.set(hub_asset.clone(), sync.clone());
        GHOST_SYNC = Some(markets);
        sync
    }
}

/// The index pair of `hub_asset`'s snapshot, so a bulk index read and a sync
/// read of one market agree the way production's single pool state does.
pub(crate) fn market_index(env: &Env, hub_asset: &HubAssetKey) -> MarketIndexRaw {
    let state = sync_data(env, hub_asset).state;
    MarketIndexRaw {
        supply_index: state.supply_index,
        borrow_index: state.borrow_index,
    }
}
