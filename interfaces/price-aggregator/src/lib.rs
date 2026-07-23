#![no_std]

//! Client-only ABI mirror of the price-aggregator contract (production surface).
//!
//! `#[contractclient]` generates `PriceAggregatorClient`. Matches deployed
//! entrypoints by ABI name (no formal `impl`). Test-only seeding
//! (`seed_oracle_config` / `remove_oracle_config`) and Ownable surface
//! (`get_owner` / `transfer_ownership` / `accept_ownership` /
//! `renounce_ownership`) are excluded.

use common::types::{AssetOracleConfig, OracleTolerance, PriceFeedRaw, PriceStatus};
use soroban_sdk::{contractclient, Address, Env, Map, Vec};

#[contractclient(name = "PriceAggregatorClient")]
pub trait PriceAggregatorInterface {
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
    fn prices(env: Env, assets: Vec<Address>) -> Map<Address, PriceFeedRaw>;

    /// Single token-rooted USD price. Fail-closed (same checks as `prices`).
    ///
    /// # Errors
    /// Same named variants as [`Self::prices`].
    fn price(env: Env, asset: Address) -> PriceFeedRaw;

    /// Soft diagnostic status for one asset. Public; never reverts on stale,
    /// dual-source deviation, or unreadable feeds — those set flags / yield
    /// [`PriceStatus::unusable`].
    fn price_status(env: Env, asset: Address) -> PriceStatus;

    /// Bulk soft diagnostic statuses (one context + multi-feed prefetch).
    /// Never reverts for stale feeds or dual-source deviation; those set flags
    /// on each [`PriceStatus`]. Unreadable feeds yield [`PriceStatus::unusable`].
    fn prices_status(env: Env, assets: Vec<Address>) -> Map<Address, PriceStatus>;

    /// Token-rooted oracle config for `asset`, if configured. Public view.
    fn oracle_config(env: Env, asset: Address) -> Option<AssetOracleConfig>;

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
    fn set_oracle_config(env: Env, asset: Address, config: AssetOracleConfig);

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
    fn set_sanity_band(env: Env, asset: Address, min_wad: i128, max_wad: i128);

    /// Updates the primary/anchor tolerance band on an active oracle. Owner only.
    ///
    /// # Errors
    /// * `OracleNotConfigured` — no stored config for `asset`.
    /// * `BadLastTolerance` — tolerance outside envelope.
    ///
    /// # Events
    /// * topics — `["config", "oracle"]`
    fn set_tolerance(env: Env, asset: Address, tolerance: OracleTolerance);
}
