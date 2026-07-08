//! Reads — SEP-40 `PriceFeedTrait` shape, delegating to the RedStone-ABI
//! reads via plain function calls (same contract, no cross-contract
//! overhead).

use common::oracle::observation::{millis_to_seconds, u256_to_i128};
use common::oracle::providers::redstone::{RedStonePriceData, REDSTONE_DECIMALS};
use common::oracle::providers::reflector::{ReflectorAsset, ReflectorPriceData};
use soroban_sdk::{contractimpl, Env, Symbol, Vec};

use crate::aggregation::MAX_HISTORY_LEN;
use crate::storage::{load_all_assets, load_feed_id, load_resolution};
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
        load_resolution(&env)
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

    /// Sample whose observation time is closest to, at or before, `timestamp`.
    pub fn price(env: Env, asset: ReflectorAsset, timestamp: u64) -> Option<ReflectorPriceData> {
        let feed_id = load_feed_id(&env, &asset)?;
        let history = Self::read_price_history(env.clone(), feed_id, MAX_HISTORY_LEN).ok()?;

        // Select on the observation time this endpoint exposes
        // (`package_timestamp`), not the write time, so a sample observed at or
        // before `timestamp` but recorded later still qualifies. History is
        // ordered by recording, and `package_timestamp` is the median's oldest
        // contributor — non-monotonic across recomputes — so scan the whole
        // window for the greatest qualifying observation rather than trusting
        // position.
        let mut best: Option<RedStonePriceData> = None;
        for entry in history.iter() {
            if millis_to_seconds(entry.package_timestamp) > timestamp {
                continue;
            }
            let closer = match &best {
                Some(b) => entry.package_timestamp > b.package_timestamp,
                None => true,
            };
            if closer {
                best = Some(entry);
            }
        }
        best.map(|entry| to_reflector_price_data(&env, &entry))
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

/// Converts wire data into the SEP-40 shape. `u256_to_i128` fails closed with a
/// panic if the price ever exceeds `i128` rather than substituting a wrong one.
///
/// The single SEP-40 `timestamp` carries the observation time
/// (`package_timestamp`), not `write_timestamp` (always ~now): exposing the
/// write time would let this path accept an aggregate built from stale
/// submissions that the RedStone path, which bounds freshness by the older
/// observation time, would reject.
fn to_reflector_price_data(env: &Env, data: &RedStonePriceData) -> ReflectorPriceData {
    let price = u256_to_i128(env, &data.price);
    let timestamp = millis_to_seconds(data.package_timestamp);
    ReflectorPriceData { price, timestamp }
}
