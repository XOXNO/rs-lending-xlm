use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::errors::{GenericError, OracleError};
use crate::oracle::observation::{
    MAX_ORACLE_DECIMALS, MAX_PRICE_STALE_SECONDS, MIN_ORACLE_DECIMALS, MIN_PRICE_STALE_SECONDS,
};
use crate::types::composable_oracle::{
    FeedSource, IndependencePolicy, LpShareSource, PoolKind, PriceKey, PriceSource, ProviderKind,
    ProviderRef, ScaledSource, SourceProperties, MAX_RESOLUTION_DEPTH, MAX_SOURCES, MIN_SOURCES,
};
use crate::types::oracle::OracleReadMode;
use crate::validation::validate_twap_records;

const MIN_ASSET_DECIMALS: u32 = 3;
const MAX_ASSET_DECIMALS: u32 = 18;

const MIN_SMOOTHING_TWAP_RECORDS: u32 = 2;

pub fn validate_source_count(env: &Env, count: u32) {
    if !(MIN_SOURCES..=MAX_SOURCES).contains(&count) {
        panic_with_error!(env, OracleError::SourceCountOutOfRange);
    }
}

pub fn validate_composition_depth(env: &Env, properties: &SourceProperties) {
    if properties.depth > MAX_RESOLUTION_DEPTH {
        panic_with_error!(env, OracleError::OracleDepthExceeded);
    }
}

pub fn validate_staleness_envelope(
    env: &Env,
    asset_max_stale_seconds: u64,
    properties: &SourceProperties,
) {
    if !(MIN_PRICE_STALE_SECONDS..=MAX_PRICE_STALE_SECONDS).contains(&asset_max_stale_seconds) {
        panic_with_error!(env, OracleError::InvalidStalenessConfig);
    }
    if properties.loosest_max_stale_seconds > asset_max_stale_seconds {
        panic_with_error!(env, OracleError::InvalidStalenessConfig);
    }
}

pub fn validate_smoothing(env: &Env, first: &SourceProperties, second: Option<&SourceProperties>) {
    let clean =
        !first.has_unsmoothed_market_leg || second.is_some_and(|s| !s.has_unsmoothed_market_leg);
    if !clean {
        panic_with_error!(env, GenericError::SpotOnlyNotProductionSafe);
    }
}

pub fn validate_independence(
    env: &Env,
    first: &SourceProperties,
    second: &SourceProperties,
    policy: &IndependencePolicy,
) {
    let shared_contracts = first.shared_contracts_with(env, second);

    match policy {
        IndependencePolicy::RequireDisjoint => {
            if !shared_contracts.is_empty() {
                panic_with_error!(env, OracleError::IndependenceNotDeclared);
            }
        }
        IndependencePolicy::AllowShared(declared) => {
            if declared.is_empty() {
                panic_with_error!(env, OracleError::IndependenceNotDeclared);
            }
            let mut declared_contracts = Vec::new(env);
            for domain in declared.iter() {
                if !declared_contracts.iter().any(|c| c == domain.contract) {
                    declared_contracts.push_back(domain.contract.clone());
                }
            }
            if !same_address_set(&shared_contracts, &declared_contracts) {
                panic_with_error!(env, OracleError::IndependenceNotDeclared);
            }
        }
    }
}

fn same_address_set(left: &Vec<Address>, right: &Vec<Address>) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter().all(|a| right.iter().any(|b| b == a))
}

pub fn validate_source_shape(env: &Env, source: &PriceSource) {
    match source {
        PriceSource::Feed(feed) => validate_feed_shape(env, feed),
        PriceSource::Scaled(scaled) => {
            validate_feed_shape(env, &scaled.factor);
            validate_factor_bounds(env, scaled);
        }
        PriceSource::LpShare(lp) => validate_lp_share_shape(env, lp),
    }
}

fn validate_lp_share_shape(env: &Env, lp: &LpShareSource) {
    match lp.kind {
        PoolKind::ConstantProduct => {}
    }
    for decimals in [lp.reserve_a_decimals, lp.reserve_b_decimals, lp.share_decimals] {
        if !(MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(&decimals) {
            panic_with_error!(env, OracleError::InvalidOracleDecimals);
        }
    }
    // The two legs must be distinct price keys, else the pair collapses.
    if lp.key_a == lp.key_b {
        panic_with_error!(env, OracleError::InvalidOracleBase);
    }
}

fn validate_feed_shape(env: &Env, feed: &FeedSource) {
    if !(MIN_ORACLE_DECIMALS..=MAX_ORACLE_DECIMALS).contains(&feed.decimals) {
        panic_with_error!(env, OracleError::InvalidOracleDecimals);
    }

    if !(MIN_PRICE_STALE_SECONDS..=MAX_PRICE_STALE_SECONDS).contains(&feed.max_stale_seconds) {
        panic_with_error!(env, OracleError::InvalidStalenessConfig);
    }
    match &feed.provider {
        ProviderRef::Reflector(reflector) => {
            if let OracleReadMode::Twap(records) = reflector.read_mode {
                validate_twap_records(env, records);
                if records < MIN_SMOOTHING_TWAP_RECORDS {
                    panic_with_error!(env, OracleError::TwapInsufficientObservations);
                }
            }
        }

        ProviderRef::MultiFeed(multi_feed) => {
            if matches!(multi_feed.kind, ProviderKind::Reflector) {
                panic_with_error!(env, GenericError::InvalidExchangeSrc);
            }
        }
    }
}

pub fn validate_asset_decimals(env: &Env, key: &PriceKey, asset_decimals: u32) {
    let ok = match key {
        PriceKey::Token(_) => (MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(&asset_decimals),
        PriceKey::Ref(_) => asset_decimals == 0,
    };
    if !ok {
        panic_with_error!(env, OracleError::InvalidOracleDecimals);
    }
}

pub fn validate_factor_bounds(env: &Env, scaled: &ScaledSource) {
    use crate::constants::MAX_REASONABLE_PRICE_WAD;
    if scaled.min_factor_wad <= 0
        || scaled.max_factor_wad < scaled.min_factor_wad
        || scaled.max_factor_wad > MAX_REASONABLE_PRICE_WAD
    {
        panic_with_error!(env, OracleError::InvalidSanityBounds);
    }
}

pub fn require_factor_in_bounds(env: &Env, factor_wad: i128, scaled: &ScaledSource) {
    if factor_wad < scaled.min_factor_wad || factor_wad > scaled.max_factor_wad {
        panic_with_error!(env, OracleError::FactorOutOfBounds);
    }
}

#[cfg(test)]
#[path = "../../tests/oracle/policy.rs"]
mod tests;
