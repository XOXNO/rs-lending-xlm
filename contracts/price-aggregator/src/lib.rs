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
mod engine;
mod events;
mod observation;
mod prefetch;
mod price;
mod properties;
mod providers;
mod registry;
mod status;
mod storage;
mod tolerance;

#[cfg(feature = "certora")]
#[path = "../../../certora/price-aggregator/spec/mod.rs"]
pub mod spec;

/// Shared fixtures for `compose::tests` and `price::hard_path_error_tests`.
/// Owned here (rather than under either test module) so the file is loaded
/// exactly once; those two test trees are siblings and cannot otherwise
/// share a `#[path]`-included module without loading it twice.
#[cfg(test)]
#[path = "../tests/oracle/support.rs"]
mod test_support;

use soroban_sdk::{contract, contractimpl, panic_with_error, Address, Env, Map, Vec};
use stellar_access::ownable::{self, Ownable};
use stellar_macros::only_owner;

use common::types::{
    AssetOracle, AssetOracleConfig, OracleTolerance, PriceFeedRaw, PriceKey, PriceStatus,
};

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
    /// * `TwapRecordsOutOfRange` — configured TWAP window above the cap.
    /// * `InvalidOracleBase` — quoted base not USD-rooted.
    /// * `InvalidOracleTokenType` — Reflector asset ref the provider cannot express.
    /// * `MathOverflow` — midpoint, normalize, or quoted-reprice overflow.
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

    /// USD price for `key` under the composable model. Fail-closed, same
    /// discipline as [`Self::price`].
    ///
    /// Resolves through the migrated config if one exists, otherwise through the
    /// legacy config lifted into the current shape, so this answers for every
    /// configured asset during migration as well as after it.
    ///
    /// # Errors
    /// * `OracleNotConfigured` - no config, migrated or legacy.
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
        let Some(oracle) = registry::resolve_oracle(&env, &key) else {
            panic_with_error!(&env, Error::OracleNotConfigured)
        };
        let resolved = engine::resolve_with(&mut cache, &oracle, 0);
        (resolved.low_wad, resolved.high_wad)
    }

    /// Oracle that would price `key`: the migrated config if one exists, else
    /// the legacy config lifted into the current shape. Public view.
    pub fn oracle_for(env: Env, key: PriceKey) -> Option<AssetOracle> {
        registry::resolve_oracle(&env, &key)
    }

    /// Which of `candidates` still resolve through the legacy reader.
    ///
    /// The guard on retiring that reader: it may only be removed once this
    /// returns empty for every listed asset. Takes an explicit list because
    /// persistent storage is not enumerable.
    pub fn unmigrated_oracles(env: Env, candidates: Vec<PriceKey>) -> Vec<PriceKey> {
        registry::unmigrated(&env, &candidates)
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
