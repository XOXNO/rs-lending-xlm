//! Oracle invariant rules: staleness, tolerance bands, and price-cache consistency.

use cvlr::macros::rule;
use cvlr::nondet::nondet;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::{
    MAX_FIRST_TOLERANCE, MAX_LAST_TOLERANCE, MIN_FIRST_TOLERANCE, MIN_LAST_TOLERANCE, WAD,
};
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
                first_upper_ratio_bps: 10_200,
                first_lower_ratio_bps: 9_800,
                last_upper_ratio_bps: 11_000,
                last_lower_ratio_bps: 9_000,
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
// `timestamp <= now + 60` assume (tautology). Real staleness is proven against
// the unsummarised compose pipeline in `oracle_compose_rules`.

/// First-band tolerance: the blended price stays within [min, max] of its inputs.
/// Which band is selected is modelled nondeterministically by the harness
/// `calculate_final_price`; the real band math is proven in `tolerance_math_rules`.
#[rule]
fn first_band_price_within_inputs(
    e: Env,
    _base_asset: Address,
    aggregator_price: i128,
    safe_price: i128,
    first_upper_bps: u32,
    first_lower_bps: u32,
) {
    cvlr_assume!(aggregator_price > 0 && aggregator_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(safe_price > 0 && safe_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(first_upper_bps >= MIN_FIRST_TOLERANCE);
    cvlr_assume!(first_upper_bps <= MAX_FIRST_TOLERANCE);
    cvlr_assume!(first_lower_bps >= MIN_FIRST_TOLERANCE);
    cvlr_assume!(first_lower_bps <= MAX_FIRST_TOLERANCE);

    let cache = crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskDecreasing);
    let tolerance = OraclePriceFluctuation {
        first_upper_ratio_bps: first_upper_bps,
        first_lower_ratio_bps: first_lower_bps,
        last_upper_ratio_bps: MAX_LAST_TOLERANCE as u32,
        last_lower_ratio_bps: MIN_LAST_TOLERANCE as u32,
    };

    let final_price = crate::oracle::calculate_final_price(
        &cache,
        Some(aggregator_price),
        Some(safe_price),
        &tolerance,
    );

    let min_price = if aggregator_price < safe_price {
        aggregator_price
    } else {
        safe_price
    };
    let max_price = if aggregator_price > safe_price {
        aggregator_price
    } else {
        safe_price
    };
    cvlr_assert!(final_price >= min_price);
    cvlr_assert!(final_price <= max_price);
}

/// Second-band tolerance: the final price stays within [min, max] of its inputs.
#[rule]
fn second_band_price_within_inputs(
    e: Env,
    _base_asset: Address,
    aggregator_price: i128,
    safe_price: i128,
    first_upper_bps: u32,
    first_lower_bps: u32,
    last_upper_bps: u32,
    last_lower_bps: u32,
) {
    cvlr_assume!(aggregator_price > 0 && aggregator_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(safe_price > 0 && safe_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(first_upper_bps >= MIN_FIRST_TOLERANCE);
    cvlr_assume!(first_upper_bps <= MAX_FIRST_TOLERANCE);
    cvlr_assume!(first_lower_bps >= MIN_FIRST_TOLERANCE);
    cvlr_assume!(first_lower_bps <= MAX_FIRST_TOLERANCE);
    cvlr_assume!(last_upper_bps >= MIN_LAST_TOLERANCE);
    cvlr_assume!(last_upper_bps <= MAX_LAST_TOLERANCE);
    cvlr_assume!(last_lower_bps >= MIN_LAST_TOLERANCE);
    cvlr_assume!(last_lower_bps <= MAX_LAST_TOLERANCE);
    cvlr_assume!(last_upper_bps >= first_upper_bps);
    cvlr_assume!(last_lower_bps >= first_lower_bps);

    let cache = crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskDecreasing);
    let tolerance = OraclePriceFluctuation {
        first_upper_ratio_bps: first_upper_bps,
        first_lower_ratio_bps: first_lower_bps,
        last_upper_ratio_bps: last_upper_bps,
        last_lower_ratio_bps: last_lower_bps,
    };

    let final_price = crate::oracle::calculate_final_price(
        &cache,
        Some(aggregator_price),
        Some(safe_price),
        &tolerance,
    );

    let min_price = if aggregator_price < safe_price {
        aggregator_price
    } else {
        safe_price
    };
    let max_price = if aggregator_price > safe_price {
        aggregator_price
    } else {
        safe_price
    };
    cvlr_assert!(final_price >= min_price);
    cvlr_assert!(final_price <= max_price);
}

/// Beyond-band tolerance: the final price stays within [min, max] of its inputs.
#[rule]
fn beyond_band_price_within_inputs(
    e: Env,
    _base_asset: Address,
    aggregator_price: i128,
    safe_price: i128,
    first_upper_bps: u32,
    first_lower_bps: u32,
    last_upper_bps: u32,
    last_lower_bps: u32,
) {
    cvlr_assume!(aggregator_price > 0 && aggregator_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(safe_price > 0 && safe_price <= MAX_REALISTIC_PRICE);
    cvlr_assume!(first_upper_bps >= MIN_FIRST_TOLERANCE);
    cvlr_assume!(first_upper_bps <= MAX_FIRST_TOLERANCE);
    cvlr_assume!(first_lower_bps >= MIN_FIRST_TOLERANCE);
    cvlr_assume!(first_lower_bps <= MAX_FIRST_TOLERANCE);
    cvlr_assume!(last_upper_bps >= MIN_LAST_TOLERANCE);
    cvlr_assume!(last_upper_bps <= MAX_LAST_TOLERANCE);
    cvlr_assume!(last_lower_bps >= MIN_LAST_TOLERANCE);
    cvlr_assume!(last_lower_bps <= MAX_LAST_TOLERANCE);

    let cache = crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskDecreasing);
    let tolerance = OraclePriceFluctuation {
        first_upper_ratio_bps: first_upper_bps,
        first_lower_ratio_bps: first_lower_bps,
        last_upper_ratio_bps: last_upper_bps,
        last_lower_ratio_bps: last_lower_bps,
    };

    let final_price = crate::oracle::calculate_final_price(
        &cache,
        Some(aggregator_price),
        Some(safe_price),
        &tolerance,
    );

    let min_price = if aggregator_price < safe_price {
        aggregator_price
    } else {
        safe_price
    };
    let max_price = if aggregator_price > safe_price {
        aggregator_price
    } else {
        safe_price
    };
    cvlr_assert!(final_price >= min_price);
    cvlr_assert!(final_price <= max_price);
}

/// On a cache hit, `token_price` returns the stored feed unchanged.
#[rule]
fn price_cache_consistency(e: Env, asset: Address) {
    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);

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

    let within = crate::oracle::is_within_anchor(&e, agg, safe, 200, 200);
    cvlr_satisfy!(within);
}

/// `token_price` can return a positive feed under Active + Single configuration.
#[rule]
fn price_cache_sanity(e: Env, asset: Address, pool: Address, oracle: Address) {
    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskDecreasing);
    let market = pinned_market_config(&e, &asset, &pool, oracle, MarketStatus::Active);
    cache.market_configs.set(asset.clone(), market);

    let feed = crate::oracle::token_price(&mut cache, &asset);
    cvlr_satisfy!(feed.price_wad > 0);
}
