//! Price aggregator: the lending protocol's single oracle entry point.
//!
//! Owns token-rooted `AssetOracle` configs and every oracle interaction
//! (source reads, composition, primary/anchor tolerance, staleness, sanity
//! bounds, recursive quote resolution). Consumers make one `prices(assets)`
//! call per transaction and use the returned map. Fail-closed: any unsafe,
//! stale, or unconfigured asset reverts, so the whole transaction dies rather
//! than a bad price being returned.

#![no_std]

mod compose;
mod config;
mod context;
mod events;
mod observation;
mod prefetch;
mod price;
mod providers;
mod storage;
mod tolerance;

use soroban_sdk::{contract, contractimpl, Address, Env, Map, Vec};
use stellar_access::ownable::{self, Ownable};
use stellar_macros::only_owner;

use common::types::{MarketOracleConfig, OraclePriceFluctuation, PriceFeedRaw};

pub use common::errors::OracleError as Error;

#[contract]
pub struct PriceAggregator;

#[contractimpl]
impl PriceAggregator {
    /// Registers `owner` (the governance contract) as the OZ `Ownable` owner.
    pub fn __constructor(env: Env, owner: Address) {
        ownable::set_owner(&env, &owner);
    }

    /// Bulk token-rooted USD prices for `assets`. Fail-closed: reverts on any
    /// unsafe, stale, or unconfigured asset, so the caller never receives a bad
    /// price. One call resolves every asset a transaction needs.
    pub fn prices(env: Env, assets: Vec<Address>) -> Map<Address, PriceFeedRaw> {
        let mut cache = context::PriceCache::new(&env);
        prefetch::prefetch_redstone_feeds(&mut cache, &assets);
        let mut out = Map::new(&env);
        for asset in assets.iter() {
            let feed = price::token_price(&mut cache, &asset);
            out.set(asset, feed);
        }
        out
    }

    /// Single token-rooted USD price (fail-closed).
    pub fn price(env: Env, asset: Address) -> PriceFeedRaw {
        let mut cache = context::PriceCache::new(&env);
        price::token_price(&mut cache, &asset)
    }

    /// Safe/aggregator price pair (primary-or-final, anchor-or-final) for the
    /// controller's read-only views layer.
    pub fn price_components(env: Env, asset: Address) -> (i128, i128) {
        let mut cache = context::PriceCache::new(&env);
        let config = cache.cached_asset_oracle(&asset);
        compose::resolve_components(&mut cache, &config).to_abi_prices()
    }

    /// Token-rooted oracle config for `asset`, if configured.
    pub fn get_asset_oracle(env: Env, asset: Address) -> Option<MarketOracleConfig> {
        storage::get_asset_oracle(&env, &asset)
    }

    /// Registers or replaces the token-rooted oracle config for `asset`.
    #[only_owner]
    pub fn set_market_oracle_config(env: Env, asset: Address, config: MarketOracleConfig) {
        config::set_market_oracle_config(&env, asset, config);
    }

    /// Walks the sanity band on an active oracle (live-price-contained).
    #[only_owner]
    pub fn set_oracle_sanity_bounds(env: Env, asset: Address, min_wad: i128, max_wad: i128) {
        config::set_oracle_sanity_bounds(&env, asset, min_wad, max_wad);
    }

    /// Updates the primary/anchor tolerance band on an active oracle.
    #[only_owner]
    pub fn set_oracle_tolerance(env: Env, asset: Address, tolerance: OraclePriceFluctuation) {
        config::set_oracle_tolerance(&env, asset, tolerance);
    }
}

/// `#[contractimpl]` can't see through to `Ownable`'s trait defaults, so each
/// body is written out. `transfer_ownership`/`renounce_ownership` gate on owner
/// auth internally — no `#[only_owner]` here.
#[contractimpl]
impl Ownable for PriceAggregator {
    fn get_owner(e: &Env) -> Option<Address> {
        ownable::get_owner(e)
    }

    fn transfer_ownership(e: &Env, new_owner: Address, live_until_ledger: u32) {
        ownable::transfer_ownership(e, &new_owner, live_until_ledger);
    }

    fn accept_ownership(e: &Env) {
        ownable::accept_ownership(e);
    }

    fn renounce_ownership(e: &Env) {
        ownable::renounce_ownership(e);
    }
}
