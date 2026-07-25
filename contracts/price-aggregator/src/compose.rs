//! Composition: one traversal that gathers every configured source into a
//! `Composition` without deciding what a failure means. The leg-level rules —
//! the soft read and the staleness flag — live here, so every renderer applies
//! the same ones.
//!
//! Callers supply a per-leg gate that runs the instant a leg is read, before
//! the next source is touched, and answers whether the traversal continues.
//! Reading a leg is soft about per-asset problems but neither free nor
//! panic-free: it costs a cross-contract call, and a config-invariant violation
//! or a provider contract that reverts at read time still reverts from inside
//! the read. So the gate is what keeps a broken primary from being outranked by
//! a later leg, and what keeps a caller that has already decided from paying
//! for — and reverting inside — a leg it would discard. Answering `true`
//! throughout yields a `Composition` carrying every leg the strategy calls for.

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

/// Every leg the strategy calls for, up to the point the gate stopped. A leg
/// that cannot be read is reported as an unreadable `Leg`, but a
/// config-invariant violation still reverts from inside the read rather than
/// reaching here.
pub(crate) struct Composition {
    pub primary: Leg,
    /// `None` when no anchor leg was read: either the strategy is `Single`, or
    /// it is dual and no anchor leg reached the composition.
    /// `dual_missing_anchor` tells the two apart.
    pub anchor: Option<Leg>,
    /// The strategy called for an anchor leg this composition does not carry —
    /// the config omits the anchor source, or the gate ended the traversal
    /// before it was read. Either way there is no second opinion to price
    /// against, which is what a renderer needs to know.
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
/// touched, and answers whether to keep going. A fail-closed caller reverts
/// from inside the gate, so it always answers `true` and rejects a broken
/// primary while the anchor is still untouched. A caller whose verdict is
/// already settled by the primary answers `false` instead, and the anchor is
/// never read.
pub(crate) fn compose<G>(
    cache: &mut ResolutionContext,
    config: &AssetOracleConfig,
    mut gate: G,
) -> Composition
where
    G: FnMut(&mut ResolutionContext, &OracleSourceConfig, &Leg) -> bool,
{
    let primary_max_stale = config
        .primary
        .max_stale_seconds(config.max_price_stale_seconds);
    let primary = read_leg(cache, &config.primary, primary_max_stale);
    let proceed = gate(cache, &config.primary, &primary);

    let dual = matches!(config.strategy, OracleStrategy::PrimaryWithAnchor);
    let anchor_config = if dual { config.anchor.as_ref() } else { None };

    match anchor_config {
        Some(anchor_config) if proceed => {
            let anchor_max_stale = anchor_config.max_stale_seconds(config.max_price_stale_seconds);
            let anchor = read_leg(cache, anchor_config, anchor_max_stale);
            // Nothing follows the anchor, so its answer has no read left to stop.
            gate(cache, anchor_config, &anchor);
            Composition {
                primary,
                anchor: Some(anchor),
                dual_missing_anchor: false,
            }
        }
        // A `Single` strategy calls for no anchor; a dual one that reaches here
        // wanted an anchor leg and has none, whether the config omitted the
        // source or the gate stopped before the read.
        _ => Composition {
            primary,
            anchor: None,
            dual_missing_anchor: dual,
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
