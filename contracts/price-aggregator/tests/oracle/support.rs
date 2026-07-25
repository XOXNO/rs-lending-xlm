//! Shared fixtures for the `compose`/`price` unit-test trees: mock RedStone
//! feed registration, single/dual/quoted oracle configs, and three Reflector
//! stubs — one that reports no last price, one that reports a fixed price, and
//! one whose reads revert.
//!
//! Included once, as `crate::test_support`, via `#[path]` on `lib.rs`.
//! `compose::tests` and `price::hard_path_error_tests` are siblings, not
//! ancestor/descendant, so neither can own this file directly without the
//! other reloading the same source a second time; both instead `use` these
//! items from the shared crate-root module.

use common::constants::WAD;
use common::errors::OracleError;
use common::oracle::providers::reflector::{ReflectorAsset, ReflectorOracle, ReflectorPriceData};
use common::types::{
    AssetOracleConfig, OracleAssetRef, OracleReadMode, OracleSourceConfig,
    OracleSourceConfigOption, OracleStrategy, OracleTolerance, RedStoneSourceConfig, ReflectorBase,
    ReflectorSourceConfig,
};
use mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient};
use soroban_sdk::{contract, contractimpl, panic_with_error, Address, Env, String, Symbol, Vec};

/// Price scale every Reflector fixture declares, matched by the stubs below.
const REFLECTOR_DECIMALS: u32 = 14;

/// One WAD unit expressed at [`REFLECTOR_DECIMALS`], so a spot read of it
/// normalizes to exactly `WAD`.
const REFLECTOR_ONE_RAW: i128 = 100_000_000_000_000;

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
            decimals: REFLECTOR_DECIMALS,
            resolution_seconds: 300,
            base: ReflectorBase::Usd,
        }),
        anchor: OracleSourceConfigOption::None,
        min_sanity_price_wad: WAD - WAD / 20,
        max_sanity_price_wad: WAD + WAD / 20,
    }
}

/// Reflector primary priced in `quote` rather than USD. This is the only shape
/// in which `price` consumes `PriceStatus::valid`: the quote leg reprices
/// through `status::resolve_price_status`, so the fail-closed path rests on the
/// soft path's verdict here and nowhere else.
pub(crate) fn reflector_quoted(
    reflector: &Address,
    asset: &Address,
    quote: &Address,
    max_stale: u64,
) -> AssetOracleConfig {
    AssetOracleConfig {
        primary: OracleSourceConfig::Reflector(ReflectorSourceConfig {
            contract: reflector.clone(),
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode: OracleReadMode::Spot,
            decimals: REFLECTOR_DECIMALS,
            resolution_seconds: 300,
            base: ReflectorBase::Quoted(quote.clone()),
        }),
        ..reflector_single(reflector, asset, max_stale)
    }
}

/// Dual config pairing a RedStone primary with a Reflector spot anchor — a
/// plain shape with no config-invariant violation in either leg, so the only
/// way it fails is at read time.
pub(crate) fn redstone_primary_reflector_anchor(
    env: &Env,
    feed: &Address,
    primary_id: &str,
    reflector: &Address,
    asset: &Address,
    max_stale: u64,
) -> AssetOracleConfig {
    AssetOracleConfig {
        anchor: OracleSourceConfigOption::Some(OracleSourceConfig::Reflector(
            ReflectorSourceConfig {
                contract: reflector.clone(),
                asset: OracleAssetRef::Stellar(asset.clone()),
                read_mode: OracleReadMode::Spot,
                decimals: REFLECTOR_DECIMALS,
                resolution_seconds: 300,
                base: ReflectorBase::Usd,
            },
        )),
        ..redstone_dual(env, feed, primary_id, "ANCHOR", max_stale, 10_500, 9_500)
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

/// Reflector-shaped stub that always reports one unit stamped at the current
/// ledger time. Spot-only: `prices` reports no history, since no fixture reads
/// TWAP from it.
#[contract]
pub(crate) struct PricedReflector;

#[contractimpl]
impl ReflectorOracle for PricedReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        Some(ReflectorPriceData {
            price: REFLECTOR_ONE_RAW,
            timestamp: env.ledger().timestamp(),
        })
    }

    fn prices(_env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        None
    }
}

/// Reflector-shaped stub whose price reads revert. Stands in for a Reflector
/// contract that is paused, archived, or upgraded to an incompatible interface:
/// the oracle config naming it is perfectly valid, and only the runtime call
/// fails. Reverting with a contract error (rather than trapping) is what a real
/// SEP-40 contract does, so callers see `Error(Contract, #216)`.
#[contract]
pub(crate) struct RevertingReflector;

#[contractimpl]
impl ReflectorOracle for RevertingReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        panic_with_error!(&env, OracleError::OracleNotConfigured)
    }

    fn prices(env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        panic_with_error!(&env, OracleError::OracleNotConfigured)
    }
}
