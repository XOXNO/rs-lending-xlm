//! Hard-path composition: required sources into a final USD price.
//! Reverts on missing, stale, or out-of-band legs; soft diagnostics live in
//! `status`.

use common::errors::OracleError;
use common::oracle::observation::is_stale;
use common::types::{AssetOracleConfig, OracleSourceConfig, OracleStrategy, OracleTolerance};
use soroban_sdk::{panic_with_error, Env};

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use crate::providers;
use crate::tolerance::{midpoint_if_in_band, midpoint_price_or_zero, within_tolerance_band};

/// Hard-path resolution result; per-leg diagnostics live in `status`.
pub(crate) struct ResolvedPrice {
    pub final_price_wad: i128,
    pub timestamp: u64,
}

pub(crate) fn resolve_components(
    cache: &mut ResolutionContext,
    config: &AssetOracleConfig,
) -> ResolvedPrice {
    let primary_max_stale = config
        .primary
        .max_stale_seconds(config.max_price_stale_seconds);
    let primary = providers::read_required_source(cache, &config.primary);
    require_fresh(cache, &primary, primary_max_stale);

    match config.strategy {
        OracleStrategy::Single => ResolvedPrice {
            final_price_wad: primary.price_wad,
            timestamp: primary.timestamp(),
        },
        OracleStrategy::PrimaryWithAnchor => {
            // Missing anchor on dual strategy fails closed with NoLastPrice (#210),
            // matching the read-time backstop.
            let anchor_config = config
                .anchor
                .as_ref()
                .unwrap_or_else(|| panic_with_error!(cache.env(), OracleError::NoLastPrice));
            let anchor_max_stale = anchor_config.max_stale_seconds(config.max_price_stale_seconds);
            let anchor = providers::read_required_source(cache, anchor_config);
            require_fresh(cache, &anchor, anchor_max_stale);

            let final_price_wad = midpoint_if_in_band(
                cache.env(),
                anchor.price_wad,
                primary.price_wad,
                &config.tolerance,
            );
            // Blend freshness is the older leg.
            let timestamp = core::cmp::min(primary.timestamp(), anchor.timestamp());

            ResolvedPrice {
                final_price_wad,
                timestamp,
            }
        }
    }
}

/// Reverts `PriceFeedStale` when the observation exceeds `max_stale`.
fn require_fresh(cache: &ResolutionContext, observation: &OracleObservation, max_stale: u64) {
    if is_stale(
        cache.ledger_timestamp_secs(),
        observation.timestamp(),
        max_stale,
    ) {
        panic_with_error!(cache.env(), OracleError::PriceFeedStale);
    }
}

/// Which provider family a leg came from. The hard path needs this to raise the
/// same error code the provider used to raise itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceKind {
    Reflector,
    MultiFeed,
}

impl SourceKind {
    /// Maps a source config to its provider family, mirroring the error choice
    /// in `providers::dispatch_required_source`.
    pub(crate) fn of(source: &OracleSourceConfig) -> Self {
        match source {
            OracleSourceConfig::Reflector(_) => SourceKind::Reflector,
            OracleSourceConfig::RedStone(_) | OracleSourceConfig::Xoxno(_) => SourceKind::MultiFeed,
        }
    }
}

/// One resolved source. `Err` carries the provider family so the hard path can
/// raise `NoLastPrice` or `InvalidTicker` exactly as before.
pub(crate) struct Leg {
    pub result: Result<OracleObservation, SourceKind>,
    pub stale: bool,
}

/// Everything both paths need, gathered in one traversal and without panicking.
pub(crate) struct Composition {
    pub primary: Leg,
    /// `None` when the strategy is `Single`.
    pub anchor: Option<Leg>,
    /// Strategy is dual but the config carries no anchor.
    pub dual_missing_anchor: bool,
}

/// Soft-reads one source; `max_stale` gates the returned `stale` flag, not
/// whether the read succeeds.
fn read_leg(cache: &mut ResolutionContext, source: &OracleSourceConfig, max_stale: u64) -> Leg {
    match providers::try_read_source(cache, source) {
        Some(observation) => {
            let stale = is_stale(
                cache.ledger_timestamp_secs(),
                observation.timestamp(),
                max_stale,
            );
            Leg {
                result: Ok(observation),
                stale,
            }
        }
        None => Leg {
            result: Err(SourceKind::of(source)),
            stale: false,
        },
    }
}

/// Resolves every configured source without deciding what a failure means.
/// `price` renders failures as panics; `status` renders them as flags.
pub(crate) fn compose(cache: &mut ResolutionContext, config: &AssetOracleConfig) -> Composition {
    let primary_max_stale = config
        .primary
        .max_stale_seconds(config.max_price_stale_seconds);
    let primary = read_leg(cache, &config.primary, primary_max_stale);

    match config.strategy {
        OracleStrategy::Single => Composition {
            primary,
            anchor: None,
            dual_missing_anchor: false,
        },
        OracleStrategy::PrimaryWithAnchor => match config.anchor.as_ref() {
            None => Composition {
                primary,
                anchor: None,
                dual_missing_anchor: true,
            },
            Some(anchor_config) => {
                let anchor_max_stale =
                    anchor_config.max_stale_seconds(config.max_price_stale_seconds);
                let anchor = read_leg(cache, anchor_config, anchor_max_stale);
                Composition {
                    primary,
                    anchor: Some(anchor),
                    dual_missing_anchor: false,
                }
            }
        },
    }
}

impl Composition {
    /// Final price and timestamp when both legs are readable and agree inside
    /// the tolerance band. Timestamp is the older leg — blend freshness is the
    /// weaker of the two.
    pub(crate) fn blended(&self, env: &Env, tolerance: &OracleTolerance) -> Option<(i128, u64)> {
        let primary = self.primary.result.as_ref().ok()?;
        match self.anchor.as_ref() {
            None if !self.dual_missing_anchor => Some((primary.price_wad, primary.timestamp())),
            None => None,
            Some(anchor_leg) => {
                let anchor = anchor_leg.result.as_ref().ok()?;
                if !within_tolerance_band(env, anchor.price_wad, primary.price_wad, tolerance) {
                    return None;
                }
                let price = midpoint_price_or_zero(env, anchor.price_wad, primary.price_wad);
                Some((price, primary.timestamp().min(anchor.timestamp())))
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/oracle/compose.rs"]
mod tests;
