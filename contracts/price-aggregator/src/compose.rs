//! Composition: one traversal that gathers every configured source into a
//! `Composition` without deciding what a failure means. The leg-level rules —
//! the soft read and the staleness flag — live here, so every renderer applies
//! the same ones.
//!
//! Callers supply a per-leg gate that runs the instant a leg is read, before
//! the next source is touched. Reading a leg is soft about per-asset problems
//! but still reverts on a config-invariant violation, so the gate is what keeps
//! a broken primary from being outranked by a later leg. A gate that returns
//! yields a `Composition` carrying every leg the strategy calls for.

use common::oracle::observation::is_stale;
use common::types::{AssetOracleConfig, OracleSourceConfig, OracleStrategy, OracleTolerance};
use soroban_sdk::Env;

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use crate::providers;
use crate::tolerance::{midpoint_price_or_zero, within_tolerance_band};

/// Which provider family a leg came from. The hard path replays the required
/// read to reproduce the provider's own error; this family is the backstop for
/// the cases where that replay returns instead of reverting.
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

/// One resolved source. `Err` carries the provider family that backs the hard
/// path's `NoLastPrice` / `InvalidTicker` choice.
pub(crate) struct Leg {
    pub result: Result<OracleObservation, SourceKind>,
    pub stale: bool,
}

/// Every leg the strategy calls for. A leg that cannot be read is reported as
/// an unreadable `Leg`, but a config-invariant violation still reverts from
/// inside the read rather than reaching here.
pub(crate) struct Composition {
    pub primary: Leg,
    /// `None` when no anchor was read: either the strategy is `Single`, or it
    /// is dual and the config carries no anchor. `dual_missing_anchor` tells
    /// the two apart.
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
///
/// `gate` runs on each leg the instant it is read and before the next source is
/// touched, so a fail-closed caller rejects a broken primary while the anchor
/// is still untouched. A caller that wants every leg regardless of failure
/// passes a gate that does nothing.
pub(crate) fn compose<G>(
    cache: &mut ResolutionContext,
    config: &AssetOracleConfig,
    mut gate: G,
) -> Composition
where
    G: FnMut(&mut ResolutionContext, &OracleSourceConfig, &Leg),
{
    let primary_max_stale = config
        .primary
        .max_stale_seconds(config.max_price_stale_seconds);
    let primary = read_leg(cache, &config.primary, primary_max_stale);
    gate(cache, &config.primary, &primary);

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
                gate(cache, anchor_config, &anchor);
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
