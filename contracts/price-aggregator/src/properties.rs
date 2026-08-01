use common::errors::OracleError;
use common::types::{FeedSource, PriceKey, PriceSource};
use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::registry;
use crate::session::Session;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceProperties {
    pub has_unsmoothed_market_leg: bool,
    pub trust: Vec<Address>,
    pub loosest_max_stale_seconds: u64,
    pub depth: u32,
}

impl SourceProperties {
    fn empty(env: &Env) -> Self {
        Self {
            has_unsmoothed_market_leg: false,
            trust: Vec::new(env),
            loosest_max_stale_seconds: 0,
            depth: 0,
        }
    }

    fn of_feed(env: &Env, feed: &FeedSource) -> Self {
        Self {
            has_unsmoothed_market_leg: feed.provider.is_unsmoothed_market_leg(),
            trust: Vec::from_array(env, [feed.provider.contract().clone()]),
            loosest_max_stale_seconds: feed.max_stale_seconds,
            depth: 0,
        }
    }

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

    pub(crate) fn trusts_exactly_as(&self, other: &Self) -> bool {
        let covered = |a: &Vec<Address>, b: &Vec<Address>| a.iter().all(|d| b.contains(&d));
        covered(&self.trust, &other.trust) && covered(&other.trust, &self.trust)
    }

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

pub(crate) struct LocalProperties {
    pub local: SourceProperties,
    pub dependencies: Vec<PriceKey>,
}

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
        PriceSource::AquariusLp(lp) => LocalProperties {
            local: SourceProperties {
                has_unsmoothed_market_leg: true,
                ..SourceProperties::empty(env)
            },
            dependencies: Vec::from_array(env, [lp.key_a.clone(), lp.key_b.clone()]),
        },
    }
}

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

pub(crate) struct ConfigProperties {
    pub first: SourceProperties,
    pub second: Option<SourceProperties>,
}

impl ConfigProperties {
    pub fn combined(&self) -> SourceProperties {
        match &self.second {
            Some(second) => self.first.join(second),
            None => self.first.clone(),
        }
    }
}

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

fn require_depth(env: &Env, depth: u32) {
    if depth > common::types::MAX_RESOLUTION_DEPTH {
        panic_with_error!(env, OracleError::OracleDepthExceeded);
    }
}

#[cfg(test)]
#[path = "../tests/oracle/properties.rs"]
mod tests;
