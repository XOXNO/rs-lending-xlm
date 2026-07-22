#![no_std]

//! Client-only ABI mirror of the price-aggregator contract (the oracle authority).
//!
//! `#[contractclient]` generates `PriceAggregatorClient`. Matches deployed
//! entrypoints by ABI name (no formal `impl`). Test-only seeding entrypoints
//! (`seed_oracle_config`/`remove_oracle_config`) are excluded — they are not part
//! of the production surface.

use common::types::{AssetOracleConfig, OracleTolerance, PriceFeedRaw, PriceStatus};
use soroban_sdk::{contractclient, Address, Env, Map, Vec};

#[contractclient(name = "PriceAggregatorClient")]
pub trait PriceAggregatorInterface {
    /// Bulk-resolves every asset in one call; fail-closed (reverts on any
    /// unconfigured, stale, or unsafe price).
    fn prices(env: Env, assets: Vec<Address>) -> Map<Address, PriceFeedRaw>;
    fn price(env: Env, asset: Address) -> PriceFeedRaw;
    /// Soft diagnostic status for one asset (flags, no stale/deviation revert).
    fn price_status(env: Env, asset: Address) -> PriceStatus;
    /// Bulk soft diagnostic statuses for multi-asset views.
    fn prices_status(env: Env, assets: Vec<Address>) -> Map<Address, PriceStatus>;
    fn oracle_config(env: Env, asset: Address) -> Option<AssetOracleConfig>;
    fn set_oracle_config(env: Env, asset: Address, config: AssetOracleConfig);
    fn set_sanity_band(env: Env, asset: Address, min_wad: i128, max_wad: i128);
    fn set_tolerance(env: Env, asset: Address, tolerance: OracleTolerance);
}
