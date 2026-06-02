//! RedStone Price Feed client surface — the canonical, only home for cross-contract
//! calls to RedStone multi-feed oracles. Owns `RedStonePriceData`, the
//! `REDSTONE_DECIMALS` constant (8), and the `RedStoneMultiFeed` `#[contractclient]`
//! trait. All production code reaches RedStone through the wrappers here.

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

/// Thin wrapper around the RedStone multi-feed client; preferred over constructing
/// `RedStonePriceFeedClient` directly. Returns `Some(data)` on success, `None` on
/// any failure (matching this provider's consumption pattern).
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
