//! Computes structural properties of oracle source compositions — trusted
//! provider contracts, unsmoothed-market-leg presence, loosest staleness bound,
//! and composition depth — by recursing through a source's dependencies.

use common::errors::OracleError;
use common::types::{FeedSource, PriceKey, PriceSource};
use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::registry;
use crate::session::Session;

/// Structural properties of a source or a composition of sources: whether an
/// unsmoothed market leg is present, the set of trusted provider contracts, the
/// loosest configured staleness bound, and the maximum composition depth
/// reached.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceProperties {
    pub has_unsmoothed_market_leg: bool,
    pub trust: Vec<Address>,
    pub loosest_max_stale_seconds: u64,
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
            depth: 0,
        }
    }

    /// Builds the `SourceProperties` of a single feed source: whether its
    /// provider counts as an unsmoothed market leg, its provider contract as
    /// the sole trusted contract, and its configured staleness bound.
    fn of_feed(env: &Env, feed: &FeedSource) -> Self {
        Self {
            has_unsmoothed_market_leg: feed.provider.is_unsmoothed_market_leg(),
            trust: Vec::from_array(env, [feed.provider.contract().clone()]),
            loosest_max_stale_seconds: feed.max_stale_seconds,
            depth: 0,
        }
    }

    /// Merges `self` with `other`: ORs the unsmoothed-market-leg flags, unions
    /// the trusted-contract sets, and takes the maximum of the staleness bound
    /// and depth.
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
            depth: self.depth.max(other.depth),
        }
    }

    /// Returns whether `self` and `other` trust exactly the same set of
    /// contracts, in either order.
    pub(crate) fn trusts_exactly_as(&self, other: &Self) -> bool {
        let covered = |a: &Vec<Address>, b: &Vec<Address>| a.iter().all(|d| b.contains(&d));
        covered(&self.trust, &other.trust) && covered(&other.trust, &self.trust)
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
/// oracles at `depth + 1`). Panics with `OracleDepthExceeded` if `depth`
/// exceeds the maximum resolution depth.
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

    properties.depth = properties.depth.max(depth);
    properties
}

/// Computes the joined `SourceProperties` across every source configured on the
/// oracle registered for `key`, tracking `key` on the session's resolution
/// stack while iterating. Panics with `OracleDepthExceeded` if `depth` exceeds
/// the maximum resolution depth, or with `OracleNotConfigured` if `key` has no
/// registered oracle.
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

/// Validates that `sources` has one or two entries, then computes the
/// `ConfigProperties` of the first source and, if present, the second.
pub(crate) fn properties_of_config(
    session: &mut Session,
    sources: &Vec<PriceSource>,
) -> ConfigProperties {
    let env = session.env().clone();
    crate::validation::source_count(&env, sources.len());

    let first = properties_of_source(session, &sources.get_unchecked(0), 0);
    let second = if sources.len() == 2 {
        Some(properties_of_source(session, &sources.get_unchecked(1), 0))
    } else {
        None
    };
    ConfigProperties { first, second }
}

/// Panics with `OracleDepthExceeded` if `depth` exceeds the maximum resolution
/// depth.
fn require_depth(env: &Env, depth: u32) {
    if depth > common::types::MAX_RESOLUTION_DEPTH {
        panic_with_error!(env, OracleError::OracleDepthExceeded);
    }
}

#[cfg(test)]
#[path = "../tests/oracle/properties.rs"]
mod tests;
