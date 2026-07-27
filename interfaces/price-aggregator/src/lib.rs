#![no_std]

//! Client-only ABI mirror of the price-aggregator contract (production surface).
//!
//! `#[contractclient]` generates `PriceAggregatorClient`. Matches deployed
//! entrypoints by ABI name (no formal `impl`). Test-only seeding
//! (`seed_asset_oracle` / `remove_asset_oracle`) and Ownable surface
//! (`get_owner` / `transfer_ownership` / `accept_ownership` /
//! `renounce_ownership`) are excluded.

use common::types::{AssetOracle, OracleTolerance, PriceFeedRaw, PriceKey, PriceStatus};
use soroban_sdk::{contractclient, Address, Env, Map, Vec};

#[contractclient(name = "PriceAggregatorClient")]
pub trait PriceAggregatorInterface {
    /// Bulk token-rooted USD prices for `assets`. Fail-closed: any unsafe,
    /// stale, or unconfigured asset reverts the whole call. Public; risk-path
    /// consumers (controller) rely on the revert.
    ///
    /// # Errors
    /// * `OracleNotConfigured` — no config stored for the asset.
    /// * `OracleCycleDetected` / `OracleDepthExceeded` — composition bounds.
    /// * `PriceFeedStale` — a feed, or a composite, past its bound.
    /// * `NoLastPrice` / `InvalidTicker` — provider reported no price.
    /// * `FactorOutOfBounds` — a scaled ratio outside its configured range.
    /// * `UnsafePriceNotAllowed` — two sources outside the tolerance band.
    /// * `SanityBoundViolated` / `InvalidPrice` — final price rejected.
    /// * `ReflectorHistoryEmpty` / `TwapInsufficientObservations` — TWAP gaps.
    /// * `SourceCountOutOfRange` — stored config holds no sources.
    /// * `UnsupportedPoolKind` — LP shares are not priceable yet.
    /// * `MathOverflow` — midpoint, normalize, or scaled-product overflow.
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

    /// USD price for `key`. Fail-closed, same discipline as [`Self::price`].
    ///
    /// The key-space form: reaches reference prices, which are priceable but
    /// never collateral and so have no address to look up.
    ///
    /// # Errors
    /// Same named variants as [`Self::prices`].
    fn price_of(env: Env, key: PriceKey) -> PriceFeedRaw;

    /// The interval the configured sources actually spanned for `key`, WAD.
    ///
    /// `(low, high)` are equal for a single-source key and are the two source
    /// prices otherwise, both having already passed the agreement and sanity
    /// bands. Published so the cost of combining by midpoint can be measured on
    /// live configs: a source compromised high moves a midpoint by half that
    /// error, where collateral valuation wants the low end and debt the high.
    ///
    /// # Errors
    /// Same named variants as [`Self::price_of`].
    fn price_spread_of(env: Env, key: PriceKey) -> (i128, i128);

    /// Stored oracle for `key`, if configured. Public view.
    fn oracle_for(env: Env, key: PriceKey) -> Option<AssetOracle>;

    /// Validates and stores a composable oracle under `key`. Owner (governance)
    /// only. Does not require a live feed at write time.
    ///
    /// # Errors
    /// * `SourceCountOutOfRange` — not one or two sources.
    /// * `OracleDepthExceeded` — composition nested past the cap.
    /// * `InvalidStalenessConfig` — ceiling out of range, or a component
    ///   permitted to outlive it.
    /// * `SpotOnlyNotProductionSafe` — every opinion is movable by trading.
    /// * `IndependenceNotDeclared` — shared trust does not match the declaration.
    /// * `InvalidSanityBounds` / `SanityBandTooWideForSingleSource` — band checks.
    /// * `InvalidOracleDecimals` — feed or asset decimals out of range.
    /// * `TwapInsufficientObservations` / `TwapRecordsOutOfRange` — TWAP window.
    /// * `UnsupportedPoolKind` — LP shares are not priceable yet.
    /// * `OracleCycleDetected` — the config names itself as a dependency.
    /// * `BadLastTolerance` — dual tolerance outside its envelope.
    ///
    /// # Events
    /// * topics — `["config", "asset_oracle"]`
    fn set_asset_oracle(env: Env, key: PriceKey, oracle: AssetOracle);

    /// Walks the sanity band on an active oracle. Owner only. The new band must
    /// overlap the old one and contain the current live price.
    ///
    /// # Errors
    /// * `OracleNotConfigured` — no stored config for `key`.
    /// * `InvalidSanityBounds` / `SanityBandTooWideForSingleSource` — band checks.
    /// * Plus every fail-closed variant from [`Self::price_of`] on the probe.
    ///
    /// # Events
    /// * topics — `["config", "asset_oracle"]`
    fn set_sanity_band(env: Env, key: PriceKey, min_wad: i128, max_wad: i128);

    /// Updates the agreement band between the two sources. Owner only.
    ///
    /// # Errors
    /// * `OracleNotConfigured` — no stored config for `key`.
    /// * `BadLastTolerance` — tolerance outside envelope.
    ///
    /// # Events
    /// * topics — `["config", "asset_oracle"]`
    fn set_tolerance(env: Env, key: PriceKey, tolerance: OracleTolerance);
}
