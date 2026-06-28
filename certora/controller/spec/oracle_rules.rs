//! Oracle invariant rules: staleness, tolerance bands, and price-cache consistency.

use cvlr::macros::rule;
use cvlr::nondet::nondet;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::{MAX_TOLERANCE, MIN_TOLERANCE, WAD};
use crate::types::{
    AssetConfigRaw, MarketConfig, MarketOracleConfig, MarketStatus, OracleAssetRef,
    OraclePriceFluctuation, OracleReadMode, OracleSourceConfig, OracleSourceConfigOption,
    OracleStrategy, PriceFeedRaw, ReflectorBase, ReflectorSourceConfig,
};

const MAX_REALISTIC_PRICE: i128 = 1_000_000 * WAD;

/// Seeds a pinned `MarketConfig` directly into the cache, bypassing storage reads.
fn pinned_market_config(
    env: &Env,
    asset: &Address,
    _pool: &Address,
    oracle: Address,
    status: MarketStatus,
) -> MarketConfig {
    MarketConfig {
        status,
        asset_config: AssetConfigRaw {
            loan_to_value_bps: 7_500,
            liquidation_threshold_bps: 8_000,
            liquidation_bonus_bps: 500,
            liquidation_fees_bps: 100,
            is_collateralizable: true,
            is_borrowable: true,
            e_mode_categories: soroban_sdk::Vec::new(env),

            is_flashloanable: true,
            flashloan_fee_bps: 9,
            asset_decimals: 7,
        },
        oracle_config: MarketOracleConfig {
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
        },
    }
}

// `price_staleness_enforced` removed: it re-asserted the harness summary's own
// `timestamp <= now + 60` assume (tautology). Real staleness is enforced by the
// unsummarised compose pipeline, which reverts on any stale required source.

/// Par ratio (100%) in BPS; tolerance bands open symmetrically around it.
const PAR_RATIO_BPS: u32 = 10_000;

/// Single-band blend property: an in-band primary/anchor pair resolves to the
/// midpoint, which always sits within [min, max] of its inputs. Out-of-band the
/// harness `calculate_final_price` reverts, so the assertion is vacuous there.
/// The real band math is proven in `tolerance_math_rules`.
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

/// Tightest band (MIN_TOLERANCE): the blended price stays within [min, max].
#[rule]
fn first_band_price_within_inputs(e: Env, aggregator_price: i128, safe_price: i128) {
    cvlr_assume!(aggregator_price > 0 && aggregator_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(safe_price > 0 && safe_price <= MAX_REALISTIC_PRICE);

    assert_blend_within_inputs(&e, aggregator_price, safe_price, MIN_TOLERANCE);
}

/// Arbitrary in-range band: the blended price stays within [min, max].
#[rule]
fn second_band_price_within_inputs(
    e: Env,
    aggregator_price: i128,
    safe_price: i128,
    tolerance_bps: u32,
) {
    cvlr_assume!(aggregator_price > 0 && aggregator_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(safe_price > 0 && safe_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(tolerance_bps >= MIN_TOLERANCE && tolerance_bps <= MAX_TOLERANCE);

    assert_blend_within_inputs(&e, aggregator_price, safe_price, tolerance_bps);
}

/// Widest band (MAX_TOLERANCE): the blended price stays within [min, max].
#[rule]
fn beyond_band_price_within_inputs(e: Env, aggregator_price: i128, safe_price: i128) {
    cvlr_assume!(aggregator_price > 0 && aggregator_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(safe_price > 0 && safe_price <= MAX_REALISTIC_PRICE);

    assert_blend_within_inputs(&e, aggregator_price, safe_price, MAX_TOLERANCE);
}

/// On a cache hit, `token_price` returns the stored feed unchanged.
#[rule]
fn price_cache_consistency(e: Env, asset: Address) {
    let mut cache = crate::cache::Cache::new(&e);

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
    cache.prices_cache.set(asset.clone(), seeded.clone());

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

/// `token_price` can return a positive feed under Active + Single configuration.
#[rule]
fn price_cache_sanity(e: Env, asset: Address, pool: Address, oracle: Address) {
    let mut cache = crate::cache::Cache::new(&e);
    let market = pinned_market_config(&e, &asset, &pool, oracle, MarketStatus::Active);
    cache.market_configs.set(asset.clone(), market);

    let feed = crate::oracle::token_price(&mut cache, &asset);
    cvlr_satisfy!(feed.price_wad > 0);
}
