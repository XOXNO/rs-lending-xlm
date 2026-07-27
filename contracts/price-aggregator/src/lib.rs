//! Price aggregator: the lending protocol's single oracle entry point.
//!
//! Owns every [`AssetOracle`] and every oracle interaction: source reads,
//! composition, agreement, staleness, sanity bounds, and recursive quote
//! resolution.
//!
//! Risk paths use `price`/`prices` (fail-closed); views use
//! `price_status`/`prices_status` (soft flags). Both render the same
//! composition under the same rules, so `valid` is true exactly when the hard
//! path would not revert.
//!
//! The address-keyed entrypoints exist for ABI compatibility with the
//! controller; internally everything is a [`PriceKey`], which also covers
//! reference prices that have no token.

#![no_std]

mod attest;
mod config;
mod context;
mod engine;
mod events;
mod observation;
mod prefetch;
mod properties;
mod providers;
mod registry;
mod tolerance;

#[cfg(feature = "certora")]
#[path = "../../../certora/price-aggregator/spec/mod.rs"]
pub mod spec;

/// Shared test fixtures. Owned at the crate root so sibling test trees can each
/// `use` them without the file being loaded twice.
#[cfg(test)]
#[path = "../tests/oracle/support.rs"]
mod test_support;

use soroban_sdk::{contract, contractimpl, Address, Env, Map, Vec};
use stellar_access::ownable::{self, Ownable};
use stellar_macros::only_owner;

use common::types::{AssetOracle, OracleTolerance, PriceFeedRaw, PriceKey, PriceStatus};

pub use common::errors::OracleError as Error;

/// Lifts an address list into the key space the engine works in.
fn token_keys(env: &Env, assets: &Vec<Address>) -> Vec<PriceKey> {
    let mut keys = Vec::new(env);
    for asset in assets.iter() {
        keys.push_back(PriceKey::Token(asset));
    }
    keys
}

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
    /// Address-keyed for ABI compatibility - the controller calls this and
    /// `prices_status` and nothing else. Each address is a `PriceKey::Token`;
    /// reference prices are reachable only through [`Self::price_of`], because
    /// they are priceable but never collateral.
    ///
    /// # Errors
    /// * `OracleNotConfigured` - no config stored for the asset.
    /// * `OracleCycleDetected` / `OracleDepthExceeded` - composition bounds.
    /// * `PriceFeedStale` - a feed, or a composite, past its bound.
    /// * `NoLastPrice` / `InvalidTicker` - provider reported no price.
    /// * `FactorOutOfBounds` - a scaled ratio outside its configured range.
    /// * `UnsafePriceNotAllowed` - two sources outside the tolerance band.
    /// * `SanityBoundViolated` / `InvalidPrice` - final price rejected.
    /// * `ReflectorHistoryEmpty` / `TwapInsufficientObservations` - TWAP gaps.
    /// * `SourceCountOutOfRange` - stored config holds no sources.
    /// * `UnsupportedPoolKind` - LP shares are not priceable yet.
    /// * `MathOverflow` - midpoint, normalize, or scaled-product overflow.
    pub fn prices(env: Env, assets: Vec<Address>) -> Map<Address, PriceFeedRaw> {
        let mut cache = context::ResolutionContext::new(&env);
        prefetch::warm_multi_feed_adapters(&mut cache, &token_keys(&env, &assets));
        let mut out = Map::new(&env);
        for asset in assets.iter() {
            let feed = engine::resolve(&mut cache, &PriceKey::Token(asset.clone()), 0);
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
        engine::resolve(&mut cache, &PriceKey::Token(asset), 0)
    }

    /// Soft diagnostic status for one asset. Never reverts on a per-asset
    /// problem - stale, deviation, and unreadable feeds set flags or yield
    /// [`PriceStatus::unusable`].
    ///
    /// `valid` is true exactly when [`Self::price`] would not revert, because
    /// both render the same composition under the same rules.
    pub fn price_status(env: Env, asset: Address) -> PriceStatus {
        let mut cache = context::ResolutionContext::new(&env);
        engine::resolve_status(&mut cache, &PriceKey::Token(asset), 0)
    }

    /// Bulk soft diagnostic statuses (one context + multi-feed prefetch).
    pub fn prices_status(env: Env, assets: Vec<Address>) -> Map<Address, PriceStatus> {
        let mut cache = context::ResolutionContext::new(&env);
        prefetch::warm_multi_feed_adapters(&mut cache, &token_keys(&env, &assets));
        let mut out = Map::new(&env);
        for asset in assets.iter() {
            let status = engine::resolve_status(&mut cache, &PriceKey::Token(asset.clone()), 0);
            out.set(asset, status);
        }
        out
    }

    /// USD price for `key` under the composable model. Fail-closed, same
    /// discipline as [`Self::price`].
    ///
    /// # Errors
    /// * `OracleNotConfigured` - no config stored for `key`.
    /// * `OracleCycleDetected` / `OracleDepthExceeded` - composition bounds.
    /// * `PriceFeedStale` - a feed, or a composed source, past its bound.
    /// * `FactorOutOfBounds` - a scaled ratio outside its configured range.
    /// * `UnsafePriceNotAllowed` - two sources outside the tolerance band.
    /// * `SanityBoundViolated` / `InvalidPrice` - final price rejected.
    /// * `UnsupportedPoolKind` - LP shares are not priceable yet.
    pub fn price_of(env: Env, key: PriceKey) -> PriceFeedRaw {
        let mut cache = context::ResolutionContext::new(&env);
        engine::resolve(&mut cache, &key, 0)
    }

    /// The interval the configured sources actually spanned for `key`, WAD.
    ///
    /// `(low, high)` are equal for a single-source key and are the two source
    /// prices otherwise, both having already passed the tolerance band and the
    /// sanity band. Published because the combination rule is the open question
    /// in this model: a source compromised high moves a midpoint by half that
    /// error, where collateral valuation wants the low end and debt the high
    /// end. Exposing the interval lets that be measured on live configs before
    /// `PriceFeedRaw` is widened to carry it.
    ///
    /// # Errors
    /// Same variants as [`Self::price_of`].
    pub fn price_spread_of(env: Env, key: PriceKey) -> (i128, i128) {
        let mut cache = context::ResolutionContext::new(&env);
        let resolved = engine::resolve_detailed(&mut cache, &key, 0);
        (resolved.low_wad, resolved.high_wad)
    }

    /// Stored oracle for `key`, if configured. Public view.
    pub fn oracle_for(env: Env, key: PriceKey) -> Option<AssetOracle> {
        registry::resolve_oracle(&env, &key)
    }

    /// Validates and stores a composable oracle under `key`. Owner (governance)
    /// only.
    ///
    /// # Errors
    /// * `SourceCountOutOfRange` - not one or two sources.
    /// * `OracleDepthExceeded` - composition nested past the cap.
    /// * `InvalidStalenessConfig` - ceiling out of range, or a component
    ///   permitted to outlive it.
    /// * `SpotOnlyNotProductionSafe` - every opinion is movable by trading.
    /// * `IndependenceNotDeclared` - shared trust does not match the declaration.
    /// * `InvalidSanityBounds` / `SanityBandTooWideForSingleSource` - band checks.
    /// * `BadLastTolerance` - dual tolerance outside its envelope.
    /// * `InvalidOracleDecimals` - feed or asset decimals out of range.
    /// * `TwapInsufficientObservations` / `TwapRecordsOutOfRange` - TWAP window.
    /// * `UnsupportedPoolKind` - LP shares are not priceable yet.
    /// * `OracleCycleDetected` - the config names itself as a dependency.
    ///
    /// # Events
    /// * topics - `["config", "asset_oracle"]`, carrying the stored config
    ///   verbatim so a declared independence waiver or a feed marked
    ///   fundamental is externally observable.
    #[only_owner]
    pub fn set_asset_oracle(env: Env, key: PriceKey, oracle: AssetOracle) {
        config::set_asset_oracle(&env, key, oracle);
    }

    /// Walks the sanity band on an active oracle. Owner only.
    ///
    /// The new band must overlap the old one and contain the current live
    /// hard-path price: a band can be walked, never teleported to a disjoint
    /// range on one transient print.
    ///
    /// # Errors
    /// * `OracleNotConfigured` — no stored config for `key`.
    /// * `InvalidSanityBounds` / `SanityBandTooWideForSingleSource` — band checks.
    /// * Plus every fail-closed variant from [`Self::price_of`] on the
    ///   containment probe.
    ///
    /// # Events
    /// * topics — `["config", "asset_oracle"]`
    #[only_owner]
    pub fn set_sanity_band(env: Env, key: PriceKey, min_wad: i128, max_wad: i128) {
        config::set_sanity_band(&env, key, min_wad, max_wad);
    }

    /// Updates the agreement band between the two sources. Owner only.
    ///
    /// # Errors
    /// * `OracleNotConfigured` — no stored config for `key`.
    /// * `BadLastTolerance` — tolerance outside envelope.
    ///
    /// # Events
    /// * topics — `["config", "asset_oracle"]`
    #[only_owner]
    pub fn set_tolerance(env: Env, key: PriceKey, tolerance: OracleTolerance) {
        config::set_tolerance(&env, key, tolerance);
    }
}

#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl PriceAggregator {
    /// Test-only: seed a config directly, bypassing owner auth and validation,
    /// so consumer tests can wire a priceable key cheaply.
    pub fn seed_asset_oracle(env: Env, key: PriceKey, oracle: AssetOracle) {
        registry::set_oracle(&env, &key, &oracle);
    }

    /// Test-only: remove a key's oracle (disables pricing for it).
    pub fn remove_asset_oracle(env: Env, key: PriceKey) {
        registry::remove_oracle(&env, &key);
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
