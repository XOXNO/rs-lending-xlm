//! Write-time dependency walk that builds [`SourceProperties`] for config
//! policy checks (depth, staleness envelope, smoothing, independence).
//!
//! Does not read live feeds. Nested keys must already be stored; missing
//! config, cycles, and excess depth panic.

use common::errors::OracleError;
use common::types::{local_properties, PriceKey, PriceSource, SourceProperties};
use soroban_sdk::{panic_with_error, Env, Vec};

use crate::admin;
use crate::session::Session;

/// Properties of one source, joining nested key dependencies at `depth + 1`.
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

/// Properties of a stored oracle for `key`, joining all of its sources.
///
/// # Errors
/// * [`OracleError::OracleNotConfigured`] — no stored oracle for `key`.
/// * [`OracleError::OracleCycleDetected`] — re-entry on the session key stack.
/// * [`OracleError::OracleDepthExceeded`] — past `MAX_RESOLUTION_DEPTH`.
pub(crate) fn properties_of_key(
    session: &mut Session,
    key: &PriceKey,
    depth: u32,
) -> SourceProperties {
    let env = session.env().clone();
    require_depth(&env, depth);

    session.push_key(key);

    let Some(oracle) = admin::get_oracle(&env, key) else {
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

/// Per-leg properties of a staged source list (1 or 2 sources).
pub(crate) struct ConfigProperties {
    pub first: SourceProperties,
    pub second: Option<SourceProperties>,
}

impl ConfigProperties {
    /// Union of both legs when dual; otherwise the single leg.
    pub fn combined(&self) -> SourceProperties {
        match &self.second {
            Some(second) => self.first.join(second),
            None => self.first.clone(),
        }
    }
}

/// Properties for a staged `sources` vector. Enforces allowed source count.
pub(crate) fn properties_of_config(
    session: &mut Session,
    sources: &Vec<PriceSource>,
) -> ConfigProperties {
    let env = session.env().clone();
    common::oracle::policy::validate_source_count(&env, sources.len());

    let first = properties_of_source(session, &sources.get_unchecked(0), 0);
    let second = if sources.len() == 2 {
        Some(properties_of_source(session, &sources.get_unchecked(1), 0))
    } else {
        None
    };
    ConfigProperties { first, second }
}

fn require_depth(env: &Env, depth: u32) {
    if depth > common::types::MAX_RESOLUTION_DEPTH {
        panic_with_error!(env, OracleError::OracleDepthExceeded);
    }
}

#[cfg(test)]
#[path = "../tests/oracle/properties.rs"]
mod tests;
