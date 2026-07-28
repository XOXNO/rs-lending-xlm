#![no_std]

//! Client ABI for the price-aggregator. All pricing uses [`PriceKey`].

use common::types::{AssetOracle, OracleTolerance, PriceFeedRaw, PriceKey, PriceStatus};
use soroban_sdk::{contractclient, Env, Map, Vec};

#[contractclient(name = "PriceAggregatorClient")]
pub trait PriceAggregatorInterface {
    /// Fail-closed bulk USD prices. Any unsafe key reverts the whole call.
    fn prices(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw>;

    /// Fail-closed single USD price.
    fn price(env: Env, key: PriceKey) -> PriceFeedRaw;

    /// Soft diagnostic for one key.
    fn quote(env: Env, key: PriceKey) -> PriceStatus;

    /// Soft bulk diagnostics.
    fn quotes(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceStatus>;

    /// Source interval (low, high) WAD after gates.
    fn price_spread(env: Env, key: PriceKey) -> (i128, i128);

    /// Stored oracle for `key`, if any.
    fn oracle(env: Env, key: PriceKey) -> Option<AssetOracle>;

    /// Validate, attest, probe, store. Owner only.
    fn set_oracle(env: Env, key: PriceKey, oracle: AssetOracle);

    /// Walk sanity band. Owner only.
    fn set_sanity_band(env: Env, key: PriceKey, min_wad: i128, max_wad: i128);

    /// Update dual-source tolerance. Owner only.
    fn set_tolerance(env: Env, key: PriceKey, tolerance: OracleTolerance);
}
