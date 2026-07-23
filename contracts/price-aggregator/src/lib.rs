//! Price aggregator: the lending protocol's single oracle entry point.
//!
//! Owns token-rooted `AssetOracle` configs and every oracle interaction
//! (source reads, composition, primary/anchor tolerance, staleness, sanity
//! bounds, recursive quote resolution). Risk paths use `price`/`prices`
//! (fail-closed). Views use `price_status`/`prices_status` (soft flags).
//! See `docs/reference/invariants.md` §4.3 and ADR 0003.

#![no_std]

mod compose;
mod config;
mod context;
mod events;
mod observation;
mod prefetch;
mod price;
mod providers;
mod status;
mod storage;
mod tolerance;

#[cfg(feature = "certora")]
#[path = "../../../certora/price-aggregator/spec/mod.rs"]
pub mod spec;

use soroban_sdk::{contract, contractimpl, Address, Env, Map, Vec};
use stellar_access::ownable::{self, Ownable};
use stellar_macros::only_owner;

use common::types::{AssetOracleConfig, OracleTolerance, PriceFeedRaw, PriceStatus};

pub use common::errors::OracleError as Error;

#[contract]
pub struct PriceAggregator;

#[contractimpl]
impl PriceAggregator {
    /// Registers `owner` (the governance contract) as the OZ `Ownable` owner.
    pub fn __constructor(env: Env, owner: Address) {
        ownable::set_owner(&env, &owner);
    }

    /// Bulk token-rooted USD prices for `assets`. Fail-closed: any unsafe,
    /// stale, or unconfigured asset reverts the whole call. Public; risk-path
    /// consumers (controller) rely on the revert.
    ///
    /// # Errors
    /// * `OracleNotConfigured` — missing or pending `AssetOracle`.
    /// * `OracleCycleDetected` — quote/anchor cycle while resolving.
    /// * `PriceFeedStale` — observation past max stale or beyond future skew.
    /// * `NoLastPrice` — Reflector spot missing, or dual strategy without anchor.
    /// * `InvalidTicker` — RedStone/Xoxno feed missing.
    /// * `UnsafePriceNotAllowed` — primary/anchor outside tolerance band.
    /// * `SanityBoundViolated` — final price outside sanity band.
    /// * `InvalidPrice` — non-positive final or invalid provider payload.
    /// * `ReflectorHistoryEmpty` / `TwapInsufficientObservations` — TWAP gaps.
    /// * `InvalidOracleBase` — quoted base not USD-rooted.
    /// * `MathOverflow` — midpoint or normalize overflow.
    pub fn prices(env: Env, assets: Vec<Address>) -> Map<Address, PriceFeedRaw> {
        let mut cache = context::ResolutionContext::new(&env);
        prefetch::warm_multi_feed_adapters(&mut cache, &assets);
        let mut out = Map::new(&env);
        for asset in assets.iter() {
            let feed = price::resolve_usd_price(&mut cache, &asset);
            out.set(asset, feed);
        }
        out
    }

    /// Single token-rooted USD price. Fail-closed (same checks as `prices`).
    ///
    /// # Errors
    /// Same named variants as [`Self::prices`].
    pub fn price(env: Env, asset: Address) -> PriceFeedRaw {
        let mut cache = context::ResolutionContext::new(&env);
        price::resolve_usd_price(&mut cache, &asset)
    }

    /// Soft diagnostic status for one asset. Public; never reverts on stale,
    /// dual-source deviation, or unreadable feeds — those set flags / yield
    /// [`PriceStatus::unusable`].
    pub fn price_status(env: Env, asset: Address) -> PriceStatus {
        let mut cache = context::ResolutionContext::new(&env);
        status::resolve_price_status(&mut cache, &asset)
    }

    /// Bulk soft diagnostic statuses (one context + multi-feed prefetch).
    /// Never reverts for stale feeds or dual-source deviation; those set flags
    /// on each [`PriceStatus`]. Unreadable feeds yield [`PriceStatus::unusable`].
    pub fn prices_status(env: Env, assets: Vec<Address>) -> Map<Address, PriceStatus> {
        let mut cache = context::ResolutionContext::new(&env);
        prefetch::warm_multi_feed_adapters(&mut cache, &assets);
        let mut out = Map::new(&env);
        for asset in assets.iter() {
            out.set(
                asset.clone(),
                status::resolve_price_status(&mut cache, &asset),
            );
        }
        out
    }

    /// Token-rooted oracle config for `asset`, if configured. Public view.
    pub fn oracle_config(env: Env, asset: Address) -> Option<AssetOracleConfig> {
        storage::get_oracle_config(&env, &asset)
    }

    /// Registers or replaces the token-rooted oracle config for `asset`.
    /// Owner (governance) only. Does not require a live feed at write time.
    ///
    /// # Errors
    /// * `InvalidSanityBounds` — non-positive or inverted band, or above cap.
    /// * `SanityBandTooWideForSingleSource` — Single band exceeds midpoint width.
    /// * `BadLastTolerance` — anchored tolerance outside envelope.
    /// * `InvalidOracleBase` — Reflector quote not USD-rooted or self-quote.
    ///
    /// # Events
    /// * topics — `["config", "oracle"]`
    #[only_owner]
    pub fn set_oracle_config(env: Env, asset: Address, config: AssetOracleConfig) {
        config::set_oracle_config(&env, asset, config);
    }

    /// Walks the sanity band on an active oracle. Owner only. New band must
    /// overlap the old one and contain the current live hard-path price.
    ///
    /// # Errors
    /// * `OracleNotConfigured` — no stored config for `asset`.
    /// * `InvalidSanityBounds` / `SanityBandTooWideForSingleSource` — band checks.
    /// * Plus every fail-closed variant from [`Self::price`] on the containment probe.
    ///
    /// # Events
    /// * topics — `["config", "oracle"]`
    #[only_owner]
    pub fn set_sanity_band(env: Env, asset: Address, min_wad: i128, max_wad: i128) {
        config::set_sanity_band(&env, asset, min_wad, max_wad);
    }

    /// Updates the primary/anchor tolerance band on an active oracle. Owner only.
    ///
    /// # Errors
    /// * `OracleNotConfigured` — no stored config for `asset`.
    /// * `BadLastTolerance` — tolerance outside envelope.
    ///
    /// # Events
    /// * topics — `["config", "oracle"]`
    #[only_owner]
    pub fn set_tolerance(env: Env, asset: Address, tolerance: OracleTolerance) {
        config::set_tolerance(&env, asset, tolerance);
    }
}

#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl PriceAggregator {
    /// Test-only: seed a resolved oracle config directly, bypassing owner auth
    /// and validation, so consumer tests can wire a priceable asset cheaply.
    pub fn seed_oracle_config(env: Env, asset: Address, config: AssetOracleConfig) {
        storage::set_oracle_config(&env, &asset, &config);
    }

    /// Test-only: remove an asset's oracle (disables pricing for it).
    pub fn remove_oracle_config(env: Env, asset: Address) {
        storage::remove_oracle_config(&env, &asset);
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
