//! Oracle invariant rules: staleness, tolerance bands, and price-cache consistency.

use cvlr::macros::rule;
use cvlr::nondet::nondet;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::{MAX_TOLERANCE, MIN_TOLERANCE, WAD};
use common::types::{
    AssetOracleConfig, OracleAssetRef, OracleReadMode, OracleSourceConfig,
    OracleSourceConfigOption, OracleStrategy, OracleTolerance, PriceFeedRaw, ReflectorBase,
    ReflectorSourceConfig,
};

const MAX_REALISTIC_PRICE: i128 = 1_000_000 * WAD;

/// Active Single Reflector-USD `AssetOracleConfig` fixture (`AssetOracle` present).
fn pinned_oracle_config(asset: &Address, oracle: Address) -> AssetOracleConfig {
    AssetOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        tolerance: OracleTolerance {
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
        // Valid Single-source band: (max-min)/(max+min) == 1_000 BPS.
        min_sanity_price_wad: 9 * WAD,
        max_sanity_price_wad: 11 * WAD,
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
    let tolerance = OracleTolerance {
        upper_ratio_bps: PAR_RATIO_BPS + tolerance_bps,
        lower_ratio_bps: PAR_RATIO_BPS - tolerance_bps,
    };

    let final_price =
        crate::tolerance::midpoint_if_in_band(e, aggregator_price, safe_price, &tolerance);

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
    let mut cache = crate::context::ResolutionContext::new(&e);

    let price_wad: i128 = nondet();
    let asset_decimals: u32 = nondet();
    let timestamp: u64 = nondet();
    cvlr_assume!(price_wad > 0 && price_wad <= MAX_REALISTIC_PRICE);
    cvlr_assume!(asset_decimals <= 27);
    let now_secs = cache.ledger_timestamp_secs();
    cvlr_assume!(timestamp <= now_secs + 60);
    let seeded = PriceFeedRaw {
        price_wad,
        asset_decimals,
        timestamp,
    };
    cache.store_price(&asset, seeded.clone());

    let feed = crate::price::resolve_usd_price(&mut cache, &asset);

    cvlr_assert!(feed.price_wad == seeded.price_wad);
    cvlr_assert!(feed.asset_decimals == seeded.asset_decimals);
    cvlr_assert!(feed.timestamp == seeded.timestamp);
}

#[rule]
fn single_price_respects_configured_sanity_bounds(e: Env, asset: Address, oracle: Address) {
    cvlr_assume!(asset != oracle);
    let config = pinned_oracle_config(&asset, oracle);
    crate::storage::set_oracle_config(&e, &asset, &config);

    let feed = crate::PriceAggregator::price(e, asset);
    cvlr_assert!(feed.price_wad >= config.min_sanity_price_wad);
    cvlr_assert!(feed.price_wad <= config.max_sanity_price_wad);
    cvlr_assert!(feed.asset_decimals == config.asset_decimals);
}

#[rule]
fn price_endpoint_sanity(e: Env, asset: Address, oracle: Address) {
    cvlr_assume!(asset != oracle);
    let config = pinned_oracle_config(&asset, oracle);
    crate::storage::set_oracle_config(&e, &asset, &config);

    let feed = crate::PriceAggregator::price(e, asset);
    cvlr_satisfy!(feed.price_wad > 0);
}

#[rule]
fn bulk_prices_contains_each_requested_asset(
    e: Env,
    first: Address,
    second: Address,
    oracle: Address,
) {
    cvlr_assume!(first != second);
    cvlr_assume!(first != oracle && second != oracle);
    crate::storage::set_oracle_config(&e, &first, &pinned_oracle_config(&first, oracle.clone()));
    crate::storage::set_oracle_config(&e, &second, &pinned_oracle_config(&second, oracle));

    let requested = soroban_sdk::vec![&e, first.clone(), second.clone()];
    let prices = crate::PriceAggregator::prices(e, requested);
    cvlr_assert!(prices.contains_key(first));
    cvlr_assert!(prices.contains_key(second));
}

#[rule]
fn missing_oracle_config_reverts(e: Env, asset: Address) {
    cvlr_assume!(crate::storage::get_oracle_config(&e, &asset).is_none());
    let _ = crate::PriceAggregator::price(e, asset);
    cvlr_assert!(false);
}

#[rule]
fn self_quoted_oracle_config_reverts(e: Env, asset: Address, oracle: Address) {
    cvlr_assume!(asset != oracle);
    let mut config = pinned_oracle_config(&asset, oracle);
    let OracleSourceConfig::Reflector(ref mut reflector) = config.primary else {
        unreachable!()
    };
    reflector.base = ReflectorBase::Quoted(asset.clone());

    crate::config::validate_oracle_config(&e, &asset, &config);
    cvlr_assert!(false);
}

#[rule]
fn invalid_sanity_bounds_revert(e: Env, asset: Address, oracle: Address) {
    cvlr_assume!(asset != oracle);
    let mut config = pinned_oracle_config(&asset, oracle);
    config.min_sanity_price_wad = 0;
    config.max_sanity_price_wad = 0;

    crate::config::validate_oracle_config(&e, &asset, &config);
    cvlr_assert!(false);
}
