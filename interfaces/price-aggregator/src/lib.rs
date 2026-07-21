#![no_std]

//! Client-only ABI mirror of the price-aggregator contract (the oracle authority).
//!
//! `#[contractclient]` generates `PriceAggregatorClient`. Matches deployed
//! entrypoints by ABI name (no formal `impl`). Test-only seeding entrypoints
//! (`seed_asset_oracle`/`remove_asset_oracle`) are excluded — they are not part
//! of the production surface.

use common::types::{MarketOracleConfig, OraclePriceFluctuation, PriceFeedRaw};
use soroban_sdk::{contractclient, Address, Env, Map, Vec};

#[contractclient(name = "PriceAggregatorClient")]
pub trait PriceAggregatorInterface {
    /// Bulk-resolves every asset in one call; fail-closed (reverts on any
    /// unconfigured, stale, or unsafe price).
    fn prices(env: Env, assets: Vec<Address>) -> Map<Address, PriceFeedRaw>;
    fn price(env: Env, asset: Address) -> PriceFeedRaw;
    /// `(final, safe, aggregator)` USD-WAD price triple.
    fn price_components(env: Env, asset: Address) -> (i128, i128, i128);
    fn get_asset_oracle(env: Env, asset: Address) -> Option<MarketOracleConfig>;
    fn set_market_oracle_config(env: Env, asset: Address, config: MarketOracleConfig);
    fn set_oracle_sanity_bounds(env: Env, asset: Address, min_wad: i128, max_wad: i128);
    fn set_oracle_tolerance(env: Env, asset: Address, tolerance: OraclePriceFluctuation);
}
