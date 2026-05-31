//! RedStone Price Feed client surface (canonical home).
//!
//! This module owns the complete external contract surface for RedStone
//! multi-feed oracles:
//!
//! - `RedStonePriceData` — the on-chain price data type.
//! - `REDSTONE_DECIMALS` constant (hard-coded to 8).
//! - `RedStoneMultiFeed` trait with `#[contractclient]`.
//!
//! All production code that needs to talk to a RedStone oracle should go
//! through types/wrappers originating here.
//!
//! The `#[allow(dead_code)]` on the trait is required — it exists only for
//! the macro to generate the client proxy.

use soroban_sdk::{contractclient, contracttype, Address, Env, Error, String, U256};

#[contracttype]
#[derive(Clone, Debug)]
pub struct RedStonePriceData {
    pub price: U256,
    pub package_timestamp: u64,
    pub write_timestamp: u64,
}

pub(crate) const REDSTONE_DECIMALS: u32 = 8;

#[contractclient(name = "RedStonePriceFeedClient")]
#[allow(dead_code)] // Required: trait exists only for the macro to generate the client proxy.
pub trait RedStoneMultiFeed {
    fn read_price_data_for_feed(env: Env, feed_id: String) -> Result<RedStonePriceData, Error>;
}

/// Thin wrapper around the RedStone multi-feed client.
/// All production code should prefer this (or higher-level wrappers) over
/// constructing `RedStonePriceFeedClient` directly.
///
/// Returns `Some(data)` on success, `None` on any failure (matching the
/// existing consumption pattern in this provider).
pub(crate) fn read_price_data(
    env: &Env,
    contract: &Address,
    feed_id: &String,
) -> Option<RedStonePriceData> {
    match RedStonePriceFeedClient::new(env, contract).try_read_price_data_for_feed(feed_id) {
        Ok(Ok(data)) => Some(data),
        _ => None,
    }
}
