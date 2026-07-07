//! Reads — SEP-40 `PriceFeedTrait` shape, delegating to the RedStone-ABI
//! reads via plain function calls (same contract, no cross-contract
//! overhead).

use common::oracle::observation::{millis_to_seconds, u256_to_i128};
use common::oracle::providers::redstone::{RedStonePriceData, REDSTONE_DECIMALS};
use common::oracle::providers::reflector::{ReflectorAsset, ReflectorPriceData};
use soroban_sdk::{contractimpl, Env, Symbol, Vec};

use crate::aggregation::MAX_HISTORY_LEN;
use crate::storage::{load_all_assets, load_feed_id, DataKey};
use crate::{XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

#[contractimpl]
impl XoxnoOracle {
    /// This oracle always quotes in USD.
    pub fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    /// Always `REDSTONE_DECIMALS`: every price is produced by this same
    /// contract's aggregation, fixed at 8 decimals. Importing the constant
    /// instead of a second literal keeps the two provably in sync.
    pub fn decimals(_env: Env) -> u32 {
        REDSTONE_DECIMALS
    }

    pub fn resolution(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::Resolution)
            .unwrap_or(0)
    }

    pub fn assets(env: Env) -> Vec<ReflectorAsset> {
        load_all_assets(&env)
    }

    /// Latest price for `asset`, or `None` if unmapped or no aggregate exists
    /// yet. SEP-40 models "no data" as `None` rather than an error, so
    /// `read_price_data_for_feed`'s errors are swallowed here rather than
    /// propagated.
    pub fn lastprice(env: Env, asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        let feed_id = load_feed_id(&env, &asset)?;
        let data = Self::read_price_data_for_feed(env.clone(), feed_id).ok()?;
        Some(to_reflector_price_data(&env, &data))
    }

    /// Price sample at or before `timestamp`, closest to it.
    pub fn price(env: Env, asset: ReflectorAsset, timestamp: u64) -> Option<ReflectorPriceData> {
        let feed_id = load_feed_id(&env, &asset)?;
        let history = Self::read_price_history(env.clone(), feed_id, MAX_HISTORY_LEN).ok()?;

        // History is newest-first, so the first entry at or before `timestamp`
        // is the closest sample not in the future.
        for entry in history.iter() {
            if millis_to_seconds(entry.write_timestamp) <= timestamp {
                return Some(to_reflector_price_data(&env, &entry));
            }
        }
        None
    }

    /// Up to `records` price samples for `asset`, newest-first, for the
    /// caller's TWAP computation.
    pub fn prices(
        env: Env,
        asset: ReflectorAsset,
        records: u32,
    ) -> Option<Vec<ReflectorPriceData>> {
        let feed_id = load_feed_id(&env, &asset)?;
        let history = Self::read_price_history(env.clone(), feed_id, records).ok()?;
        if history.is_empty() {
            return None;
        }
        let mut out = Vec::new(&env);
        for entry in history.iter() {
            out.push_back(to_reflector_price_data(&env, &entry));
        }
        Some(out)
    }
}

/// Converts internal wire data into the SEP-40 shape: `U256` price narrowed
/// to `i128` via the shared, already-tested `u256_to_i128` (fails closed with
/// a panic if the value ever doesn't fit, rather than silently substituting a
/// wrong price), and `write_timestamp` from milliseconds to seconds
/// (Reflector's `ReflectorPriceData.timestamp` is second-resolution).
fn to_reflector_price_data(env: &Env, data: &RedStonePriceData) -> ReflectorPriceData {
    let price = u256_to_i128(env, &data.price);
    let timestamp = millis_to_seconds(data.write_timestamp);
    ReflectorPriceData { price, timestamp }
}
