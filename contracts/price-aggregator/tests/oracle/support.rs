//! Shared fixtures for the `compose`/`price` unit-test trees: mock RedStone
//! feed registration, single/dual oracle configs, and a Reflector stub that
//! always reports no last price.
//!
//! Included once, as `crate::test_support`, via `#[path]` on `lib.rs`.
//! `compose::tests` and `price::hard_path_error_tests` are siblings, not
//! ancestor/descendant, so neither can own this file directly without the
//! other reloading the same source a second time; both instead `use` these
//! items from the shared crate-root module.

use common::constants::WAD;
use common::oracle::providers::reflector::{ReflectorAsset, ReflectorOracle, ReflectorPriceData};
use common::types::{
    AssetOracleConfig, OracleAssetRef, OracleReadMode, OracleSourceConfig,
    OracleSourceConfigOption, OracleStrategy, OracleTolerance, RedStoneSourceConfig, ReflectorBase,
    ReflectorSourceConfig,
};
use mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient};
use soroban_sdk::{contract, contractimpl, Address, Env, String, Symbol, Vec};

pub(crate) fn register_redstone_feed(env: &Env) -> (Address, MockRedStonePriceFeedClient<'_>) {
    let id = env.register(MockRedStonePriceFeed, ());
    (id.clone(), MockRedStonePriceFeedClient::new(env, &id))
}

pub(crate) fn redstone_single(
    env: &Env,
    feed: &Address,
    feed_id: &str,
    max_stale: u64,
) -> AssetOracleConfig {
    AssetOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: max_stale,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_000,
            lower_ratio_bps: 10_000,
        },
        strategy: OracleStrategy::Single,
        primary: OracleSourceConfig::RedStone(RedStoneSourceConfig {
            contract: feed.clone(),
            feed_id: String::from_str(env, feed_id),
            decimals: 8,
            max_stale_seconds: max_stale,
        }),
        anchor: OracleSourceConfigOption::None,
        min_sanity_price_wad: WAD - WAD / 20,
        max_sanity_price_wad: WAD + WAD / 20,
    }
}

pub(crate) fn redstone_dual(
    env: &Env,
    feed: &Address,
    primary_id: &str,
    anchor_id: &str,
    max_stale: u64,
    upper_bps: u32,
    lower_bps: u32,
) -> AssetOracleConfig {
    AssetOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: max_stale,
        tolerance: OracleTolerance {
            upper_ratio_bps: upper_bps,
            lower_ratio_bps: lower_bps,
        },
        strategy: OracleStrategy::PrimaryWithAnchor,
        primary: OracleSourceConfig::RedStone(RedStoneSourceConfig {
            contract: feed.clone(),
            feed_id: String::from_str(env, primary_id),
            decimals: 8,
            max_stale_seconds: max_stale,
        }),
        anchor: OracleSourceConfigOption::Some(OracleSourceConfig::RedStone(
            RedStoneSourceConfig {
                contract: feed.clone(),
                feed_id: String::from_str(env, anchor_id),
                decimals: 8,
                max_stale_seconds: max_stale,
            },
        )),
        min_sanity_price_wad: WAD / 2,
        max_sanity_price_wad: WAD * 2,
    }
}

pub(crate) fn reflector_single(
    reflector: &Address,
    asset: &Address,
    max_stale: u64,
) -> AssetOracleConfig {
    AssetOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: max_stale,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_000,
            lower_ratio_bps: 10_000,
        },
        strategy: OracleStrategy::Single,
        primary: OracleSourceConfig::Reflector(ReflectorSourceConfig {
            contract: reflector.clone(),
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode: OracleReadMode::Spot,
            decimals: 14,
            resolution_seconds: 300,
            base: ReflectorBase::Usd,
        }),
        anchor: OracleSourceConfigOption::None,
        min_sanity_price_wad: WAD - WAD / 20,
        max_sanity_price_wad: WAD + WAD / 20,
    }
}

/// Reflector-shaped stub that always reports no last price. A genuinely
/// registered contract is required for the unreadable-leg cases: an
/// unregistered address traps with a host `InvalidAction` error, not the
/// provider's own `NoLastPrice`.
#[contract]
pub(crate) struct EmptyReflector;

#[contractimpl]
impl ReflectorOracle for EmptyReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        14
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(_env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        None
    }

    fn prices(_env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        None
    }
}
