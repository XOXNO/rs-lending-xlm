//! Oracle invariant rules: staleness, tolerance bands, and price-cache consistency.

use cvlr::macros::rule;
use cvlr::nondet::nondet;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::{MAX_TOLERANCE, MIN_TOLERANCE, WAD};
use crate::types::{
    MarketOracleConfig, OracleAssetRef, OraclePriceFluctuation, OracleReadMode, OracleSourceConfig,
    OracleSourceConfigOption, OracleStrategy, PriceFeedRaw, ReflectorBase, ReflectorSourceConfig,
};

const MAX_REALISTIC_PRICE: i128 = 1_000_000 * WAD;

/// Active Single Reflector-USD `MarketOracleConfig` fixture (`AssetOracle` present).
fn pinned_oracle_config(asset: &Address, oracle: Address) -> MarketOracleConfig {
    MarketOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        tolerance: OraclePriceFluctuation {
            upper_ratio_bps: 10_200,
            lower_ratio_bps: 9_800,
        },
        strategy: OracleStrategy::Single,
        primary: OracleSourceConfig::Reflector(ReflectorSourceConfig {
            contract: oracle,
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode: OracleReadMode::Spot,
            decimals: 14,
            resolution_seconds: 300,
            base: ReflectorBase::Usd,
        }),
        anchor: OracleSourceConfigOption::None,
        min_sanity_price_wad: 0,
        max_sanity_price_wad: 0,
    }
}

/// Par ratio (100%) in BPS; bands open symmetrically around it.
const PAR_RATIO_BPS: u32 = 10_000;

/// In-band primary/anchor blend is the midpoint, within [min, max] of inputs.
/// Out-of-band the harness reverts (assertion vacuous). Exact band math: `tolerance_math_rules`.
fn assert_blend_within_inputs(
    e: &Env,
    aggregator_price: i128,
    safe_price: i128,
    tolerance_bps: u32,
) {
    let tolerance = OraclePriceFluctuation {
        upper_ratio_bps: PAR_RATIO_BPS + tolerance_bps,
        lower_ratio_bps: PAR_RATIO_BPS - tolerance_bps,
    };

    let final_price =
        crate::oracle::calculate_final_price(e, aggregator_price, safe_price, &tolerance);

    let min_price = aggregator_price.min(safe_price);
    let max_price = aggregator_price.max(safe_price);
    cvlr_assert!(final_price >= min_price);
    cvlr_assert!(final_price <= max_price);
}

#[rule]
fn first_band_price_within_inputs(e: Env, aggregator_price: i128, safe_price: i128) {
    cvlr_assume!(aggregator_price > 0 && aggregator_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(safe_price > 0 && safe_price <= MAX_REALISTIC_PRICE);

    assert_blend_within_inputs(&e, aggregator_price, safe_price, MIN_TOLERANCE);
}

#[rule]
fn second_band_price_within_inputs(
    e: Env,
    aggregator_price: i128,
    safe_price: i128,
    tolerance_bps: u32,
) {
    cvlr_assume!(aggregator_price > 0 && aggregator_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(safe_price > 0 && safe_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!((MIN_TOLERANCE..=MAX_TOLERANCE).contains(&tolerance_bps));

    assert_blend_within_inputs(&e, aggregator_price, safe_price, tolerance_bps);
}

#[rule]
fn beyond_band_price_within_inputs(e: Env, aggregator_price: i128, safe_price: i128) {
    cvlr_assume!(aggregator_price > 0 && aggregator_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(safe_price > 0 && safe_price <= MAX_REALISTIC_PRICE);

    assert_blend_within_inputs(&e, aggregator_price, safe_price, MAX_TOLERANCE);
}

#[rule]
fn price_cache_consistency(e: Env, asset: Address) {
    let mut cache = crate::context::Cache::new(&e);

    let price_wad: i128 = nondet();
    let asset_decimals: u32 = nondet();
    let timestamp: u64 = nondet();
    cvlr_assume!(price_wad > 0 && price_wad <= MAX_REALISTIC_PRICE);
    cvlr_assume!(asset_decimals <= 27);
    let now_secs = cache.current_timestamp_ms / 1000;
    cvlr_assume!(timestamp <= now_secs + 60);
    let seeded = PriceFeedRaw {
        price_wad,
        asset_decimals,
        timestamp,
    };
    cache.token_prices.set(asset.clone(), seeded.clone());

    let feed = crate::oracle::token_price(&mut cache, &asset);

    cvlr_assert!(feed.price_wad == seeded.price_wad);
    cvlr_assert!(feed.asset_decimals == seeded.asset_decimals);
    cvlr_assert!(feed.timestamp == seeded.timestamp);
}

#[rule]
fn oracle_tolerance_sanity(e: Env) {
    let agg: i128 = nondet();
    let safe: i128 = nondet();
    cvlr_assume!(agg > 0 && agg <= MAX_REALISTIC_PRICE);
    cvlr_assume!(safe > 0 && safe <= MAX_REALISTIC_PRICE);

    let tolerance = OraclePriceFluctuation {
        upper_ratio_bps: 10_200,
        lower_ratio_bps: 9_800,
    };
    let final_price = crate::oracle::calculate_final_price(&e, agg, safe, &tolerance);
    cvlr_satisfy!(final_price > 0);
}

#[rule]
fn price_cache_sanity(e: Env, asset: Address, oracle: Address) {
    let mut cache = crate::context::Cache::new(&e);
    crate::storage::set_asset_oracle(&e, &asset, &pinned_oracle_config(&asset, oracle));

    let feed = crate::oracle::token_price(&mut cache, &asset);
    cvlr_satisfy!(feed.price_wad > 0);
}
