//! Computes structural properties of oracle source compositions — trusted
//! provider contracts, unsmoothed-market-leg presence, loosest staleness bound,
//! and composition depth — by recursing through a source's dependencies.

use common::constants::{BPS, WAD};
use common::errors::OracleError;
use common::types::{
    FeedNature, FeedSource, OracleTolerance, PriceKey, PriceSource, MAX_RESOLUTION_DEPTH,
};
use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::registry;
use crate::session::Session;
use crate::validation;

/// Structural properties of a source or a composition of sources: whether an
/// unsmoothed market leg is present, the set of trusted provider contracts, the
/// loosest configured staleness bound, the loosest bound among the
/// market-nature inputs that date the composition for the leg-spread rule
/// (`None` when it has none), and the maximum composition depth reached.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceProperties {
    pub has_unsmoothed_market_leg: bool,
    pub trust: Vec<Address>,
    pub loosest_max_stale_seconds: u64,
    pub loosest_market_max_stale_seconds: Option<u64>,
    pub depth: u32,
}

impl SourceProperties {
    /// Returns a zero-valued `SourceProperties` with no trusted contracts, no
    /// unsmoothed leg, zero staleness bound, and zero depth.
    fn empty(env: &Env) -> Self {
        Self {
            has_unsmoothed_market_leg: false,
            trust: Vec::new(env),
            loosest_max_stale_seconds: 0,
            loosest_market_max_stale_seconds: None,
            depth: 0,
        }
    }

    /// Builds the `SourceProperties` of a single feed source: whether its
    /// provider counts as an unsmoothed market leg, its provider contract as
    /// the sole trusted contract, and its configured staleness bound, which is
    /// also its market bound when the feed is market-nature.
    fn of_feed(env: &Env, feed: &FeedSource) -> Self {
        Self {
            has_unsmoothed_market_leg: feed.provider.is_unsmoothed_market_leg(),
            trust: Vec::from_array(env, [feed.provider.contract().clone()]),
            loosest_max_stale_seconds: feed.max_stale_seconds,
            loosest_market_max_stale_seconds: (feed.provider.nature() == FeedNature::Market)
                .then_some(feed.max_stale_seconds),
            depth: 0,
        }
    }

    /// Merges `self` with `other`: ORs the unsmoothed-market-leg flags, unions
    /// the trusted-contract sets, and takes the maximum of the staleness
    /// bounds and depth.
    fn join(&self, other: &Self) -> Self {
        let mut trust = self.trust.clone();
        for contract in other.trust.iter() {
            if !trust.contains(&contract) {
                trust.push_back(contract);
            }
        }
        Self {
            has_unsmoothed_market_leg: self.has_unsmoothed_market_leg
                || other.has_unsmoothed_market_leg,
            trust,
            loosest_max_stale_seconds: self
                .loosest_max_stale_seconds
                .max(other.loosest_max_stale_seconds),
            loosest_market_max_stale_seconds: self
                .loosest_market_max_stale_seconds
                .max(other.loosest_market_max_stale_seconds),
            depth: self.depth.max(other.depth),
        }
    }

    /// Returns the contracts trusted by both `self` and `other`, without
    /// duplicates.
    pub(crate) fn shared_contracts_with(&self, env: &Env, other: &Self) -> Vec<Address> {
        let mut shared = Vec::new(env);
        for contract in self.trust.iter() {
            if other.trust.contains(&contract) && !shared.contains(&contract) {
                shared.push_back(contract);
            }
        }
        shared
    }
}

/// A source's own (non-recursive) properties paired with the `PriceKey`s it
/// depends on, without resolving those dependencies.
pub(crate) struct LocalProperties {
    pub local: SourceProperties,
    pub dependencies: Vec<PriceKey>,
}

/// Computes the local `SourceProperties` and dependency keys of `source`
/// without recursing into those dependencies: a plain feed has no
/// dependencies; a scaled source depends on its quote key; an Aquarius LP
/// source (standard or stable) is marked as an unsmoothed market leg and
/// depends on both of its paired keys.
pub(crate) fn local_properties(env: &Env, source: &PriceSource) -> LocalProperties {
    match source {
        PriceSource::Feed(feed) => LocalProperties {
            local: SourceProperties::of_feed(env, feed),
            dependencies: Vec::new(env),
        },
        PriceSource::Scaled(scaled) => LocalProperties {
            local: SourceProperties::of_feed(env, &scaled.factor),
            dependencies: Vec::from_array(env, [scaled.quote.clone()]),
        },
        PriceSource::AquariusLp(lp) | PriceSource::AquariusStableLp(lp) => LocalProperties {
            local: SourceProperties {
                has_unsmoothed_market_leg: true,
                ..SourceProperties::empty(env)
            },
            dependencies: Vec::from_array(env, [lp.key_a.clone(), lp.key_b.clone()]),
        },
    }
}

/// Computes the full `SourceProperties` of `source`, joining in the properties
/// of every dependency it references (recursing through their registered
/// oracles at `depth + 1`). An Aquarius LP reads as market-nature and is
/// dated by its legs' prices, so its market bound is the loosest bound of its
/// whole composition. Panics with `OracleDepthExceeded` if `depth` exceeds the
/// maximum resolution depth.
pub(crate) fn properties_of_source(
    session: &mut Session,
    source: &PriceSource,
    depth: u32,
) -> SourceProperties {
    let env = session.env().clone();
    require_depth(&env, depth);

    let described = local_properties(&env, source);
    let mut properties = described.local;

    for key in described.dependencies.iter() {
        let dependency = properties_of_key(session, &key, depth + 1);
        properties = properties.join(&dependency);
    }

    if source.is_aquarius_lp() {
        properties.loosest_market_max_stale_seconds = Some(properties.loosest_max_stale_seconds);
    }
    properties.depth = properties.depth.max(depth);
    properties
}

/// Computes the joined `SourceProperties` across every source configured on the
/// oracle registered for `key`, tracking `key` on the session's resolution
/// stack while iterating. Panics with `OracleDepthExceeded` if `depth` exceeds
/// the maximum resolution depth, `OracleCycleDetected` if `key` is already on
/// the stack, or `OracleNotConfigured` if `key` has no registered oracle.
pub(crate) fn properties_of_key(
    session: &mut Session,
    key: &PriceKey,
    depth: u32,
) -> SourceProperties {
    let env = session.env().clone();
    require_depth(&env, depth);

    session.push_key(key);

    let Some(oracle) = registry::get_oracle(&env, key) else {
        panic_with_error!(&env, OracleError::OracleNotConfigured)
    };

    let mut joined = SourceProperties::empty(&env);
    for source in oracle.sources.iter() {
        let source_properties = properties_of_source(session, &source, depth);
        joined = joined.join(&source_properties);
    }

    session.pop_key();
    joined
}

/// The properties of an oracle configuration's first source and, when present,
/// its second source.
pub(crate) struct ConfigProperties {
    pub first: SourceProperties,
    pub second: Option<SourceProperties>,
}

impl ConfigProperties {
    /// Returns the joined properties of both sources, or just `first`'s
    /// properties when there is no second source.
    pub fn combined(&self) -> SourceProperties {
        match &self.second {
            Some(second) => self.first.join(second),
            None => self.first.clone(),
        }
    }
}

/// Panics with `SourceCountOutOfRange` unless `sources` has one or two
/// entries, then returns the properties of each source as `ConfigProperties`.
pub(crate) fn properties_of_config(
    session: &mut Session,
    sources: &Vec<PriceSource>,
) -> ConfigProperties {
    let env = session.env().clone();
    validation::source_count(&env, sources.len());

    let first = properties_of_source(session, &sources.get_unchecked(0), 0);
    let second = if sources.len() == 2 {
        Some(properties_of_source(session, &sources.get_unchecked(1), 0))
    } else {
        None
    };
    ConfigProperties { first, second }
}

/// Upper bound on how far `contract` alone can move the price that `sources`
/// blend under `tolerance`: the largest ratio, in WAD, between two prices it
/// can produce while every other input holds still. `None` means unbounded.
///
/// A feed served by `contract` is unbounded. A scaled factor it serves moves
/// within `max_factor_wad / min_factor_wad`, times its reach into the quote.
/// A nested key's price cannot leave its sanity band. An LP moves no further
/// than its more exposed leg. Of two legs, the midpoint follows the less
/// exposed one within the tolerance, `(BPS + upper) / (BPS + lower)`, and
/// never exceeds the more exposed one. Every ratio rounds up.
///
/// Reads only registered oracles; `properties_of_config` has already proved
/// the composition acyclic and within depth.
pub(crate) fn contract_reach(
    env: &Env,
    contract: &Address,
    sources: &Vec<PriceSource>,
    tolerance: &OracleTolerance,
) -> Option<i128> {
    let first = source_reach(env, contract, &sources.get_unchecked(0));
    if sources.len() == 1 {
        return first;
    }
    let second = source_reach(env, contract, &sources.get_unchecked(1));
    let (less, more) = match (first, second) {
        (Some(first), Some(second)) => (Some(first.min(second)), Some(first.max(second))),
        (Some(bounded), None) | (None, Some(bounded)) => (Some(bounded), None),
        (None, None) => (None, None),
    };
    let tolerance_ratio = ratio_up(
        WAD,
        BPS + i128::from(tolerance.upper_ratio_bps),
        BPS + i128::from(tolerance.lower_ratio_bps),
    );
    let followed = less.map(|less| ratio_up(less, tolerance_ratio, WAD));
    match (followed, more) {
        (Some(followed), Some(more)) => Some(followed.min(more)),
        (followed, more) => followed.or(more),
    }
}

/// `contract`'s reach into a single source; see [`contract_reach`].
fn source_reach(env: &Env, contract: &Address, source: &PriceSource) -> Option<i128> {
    match source {
        PriceSource::Feed(feed) => (feed.provider.contract() != contract).then_some(WAD),
        PriceSource::Scaled(scaled) => {
            let factor = if scaled.factor.provider.contract() == contract {
                ratio_up(scaled.max_factor_wad, WAD, scaled.min_factor_wad)
            } else {
                WAD
            };
            key_reach(env, contract, &scaled.quote).map(|quote| ratio_up(factor, quote, WAD))
        }
        PriceSource::AquariusLp(lp) | PriceSource::AquariusStableLp(lp) => {
            key_reach(env, contract, &lp.key_a).max(key_reach(env, contract, &lp.key_b))
        }
    }
}

/// `contract`'s reach into the price registered for `key`, capped by that
/// key's sanity band.
fn key_reach(env: &Env, contract: &Address, key: &PriceKey) -> Option<i128> {
    let Some(oracle) = registry::get_oracle(env, key) else {
        panic_with_error!(env, OracleError::OracleNotConfigured)
    };
    let band = ratio_up(
        oracle.max_sanity_price_wad,
        WAD,
        oracle.min_sanity_price_wad,
    );
    Some(
        contract_reach(env, contract, &oracle.sources, &oracle.tolerance)
            .map_or(band, |reach| reach.min(band)),
    )
}

/// `x * y / d` for positive operands, rounded up, saturating at `i128::MAX`
/// on overflow or a nonpositive divisor so that an unrepresentable reach
/// counts as unbounded.
fn ratio_up(x: i128, y: i128, d: i128) -> i128 {
    if d <= 0 {
        return i128::MAX;
    }
    x.checked_mul(y)
        .and_then(|product| product.checked_add(d - 1))
        .map_or(i128::MAX, |rounded| rounded / d)
}

/// Panics with `OracleDepthExceeded` if `depth` exceeds the maximum resolution
/// depth.
fn require_depth(env: &Env, depth: u32) {
    if depth > MAX_RESOLUTION_DEPTH {
        panic_with_error!(env, OracleError::OracleDepthExceeded);
    }
}

#[cfg(test)]
#[path = "../tests/oracle/properties.rs"]
mod tests;
