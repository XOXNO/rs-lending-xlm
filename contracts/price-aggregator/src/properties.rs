//! Recursive [`SourceProperties`] computation over the registry.
//!
//! `common::oracle::policy` owns the rules and is deliberately storage-free; it
//! decides what a set of properties permits. This module is the other half: it
//! walks [`PriceKey`] dependencies to build those properties, and owns the two
//! bounds that make the walk safe.
//!
//! Both bounds are needed and neither implies the other. The cycle stack catches
//! `A -> B -> A`, which never terminates. The depth cap catches an acyclic graph
//! that does terminate but costs more CPU than a liquidation can afford — and a
//! price path that runs out of budget is a position that cannot be liquidated.

use common::errors::OracleError;
use common::types::{local_properties, PriceKey, PriceSource, SourceProperties};
use soroban_sdk::{panic_with_error, Env, Vec};

use crate::context::ResolutionContext;
use crate::registry;

/// Properties of one source, folding in every key it depends on.
///
/// `depth` is the level this source sits at; dependencies are computed one
/// deeper. A caller starting at a config's top-level source passes 0.
pub(crate) fn properties_of_source(
    cache: &mut ResolutionContext,
    source: &PriceSource,
    depth: u32,
) -> SourceProperties {
    let env = cache.env().clone();
    require_depth_within_cap(&env, depth);

    let described = local_properties(&env, source);
    let mut properties = described.local;

    for key in described.dependencies.iter() {
        let dependency = properties_of_key(cache, &key, depth + 1);
        properties = properties.join(&dependency);
    }

    // Depth is the deepest point actually reached, not the level asked for, so a
    // rule reading it sees the true cost of the tree.
    if properties.depth < depth {
        properties.depth = depth;
    }
    properties
}

/// Properties of every source configured for `key`, joined.
///
/// A dependency contributes all of its own trust and defects regardless of how
/// its opinions are combined: pricing `key` may consult either source, so
/// whatever either can be wrong about is inherited.
pub(crate) fn properties_of_key(
    cache: &mut ResolutionContext,
    key: &PriceKey,
    depth: u32,
) -> SourceProperties {
    let env = cache.env().clone();
    require_depth_within_cap(&env, depth);

    // Push before reading, not after. A guard installed after resolution would
    // never see the re-entry it exists to catch.
    cache.push_price_key(key);

    let Some(oracle) = registry::resolve_oracle(&env, key) else {
        panic_with_error!(&env, OracleError::OracleNotConfigured)
    };

    let mut joined = SourceProperties::empty(&env);
    for source in oracle.sources.iter() {
        let source_properties = properties_of_source(cache, &source, depth);
        joined = joined.join(&source_properties);
    }

    cache.pop_price_key();
    joined
}

/// Properties of each source in a config, with the one-or-two arity in the type.
///
/// [`SourceProperties`] is deliberately not a `#[contracttype]` — it is derived,
/// never stored or transmitted — so it cannot live in a Soroban `Vec`. Rather
/// than work around that, the shape states what the model already guarantees:
/// exactly one opinion, or exactly two.
pub(crate) struct ConfigProperties {
    pub first: SourceProperties,
    pub second: Option<SourceProperties>,
}

impl ConfigProperties {
    /// Everything either source depends on, joined.
    pub fn combined(&self) -> SourceProperties {
        match &self.second {
            Some(second) => self.first.join(second),
            None => self.first.clone(),
        }
    }
}

/// Computes properties for every source of a config being validated.
///
/// # Errors
/// * [`OracleError::SourceCountOutOfRange`] - not one or two sources.
pub(crate) fn properties_of_config(
    cache: &mut ResolutionContext,
    sources: &Vec<PriceSource>,
) -> ConfigProperties {
    let env = cache.env().clone();
    common::oracle::policy::validate_source_count(&env, sources.len());

    let first = properties_of_source(cache, &sources.get_unchecked(0), 0);
    let second = if sources.len() == 2 {
        Some(properties_of_source(cache, &sources.get_unchecked(1), 0))
    } else {
        None
    };
    ConfigProperties { first, second }
}

fn require_depth_within_cap(env: &Env, depth: u32) {
    if depth > common::types::MAX_RESOLUTION_DEPTH {
        panic_with_error!(env, OracleError::OracleDepthExceeded);
    }
}

#[cfg(test)]
#[path = "../tests/oracle/properties.rs"]
mod tests;
