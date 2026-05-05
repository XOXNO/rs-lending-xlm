pub mod reflector;

use common::errors::{GenericError, OracleError};
use common::fp::{Ray, Wad};
use common::fp_core;
use common::rates::simulate_update_indexes;
use common::types::{
    ExchangeSource, MarketConfig, MarketIndex, MarketStatus, OracleProviderConfig, OracleType,
    PriceFeed, ReflectorAssetKind,
};
use reflector::{ReflectorAsset, ReflectorClient};
use soroban_sdk::{panic_with_error, Address, Env};

use crate::cache::ControllerCache;

// ---------------------------------------------------------------------------
// Core dispatcher
// ---------------------------------------------------------------------------

crate::summarized!(
    token_price_summary,
    pub fn token_price(cache: &mut ControllerCache, asset: &Address) -> PriceFeed {
        if let Some(feed) = cache.try_get_price(asset) {
            return feed;
        }

        let market = cache.cached_market_config(asset);
        match market.status {
            MarketStatus::PendingOracle => {
                panic_with_error!(cache.env(), GenericError::PairNotActive);
            }
            MarketStatus::Disabled if !cache.allow_disabled_market_price => {
                panic_with_error!(cache.env(), GenericError::PairNotActive);
            }
            _ => {}
        }

        let config = market.oracle_config;
        if config.oracle_type == OracleType::None {
            panic_with_error!(cache.env(), GenericError::PairNotActive);
        }

        let price = find_price_feed(cache, &config, asset);
        if price <= 0 {
            panic_with_error!(cache.env(), OracleError::InvalidPrice);
        }
        let feed = PriceFeed {
            price_wad: price,
            asset_decimals: config.asset_decimals,
            timestamp: cache.current_timestamp_ms / 1000,
        };
        // Redundant guard: fetch helpers already call `check_not_future` on the
        // source feed; the cache-clock timestamp built here satisfies it trivially.
        check_not_future(cache, feed.timestamp);

        cache.set_price(asset, &feed);
        feed
    }
);

fn find_price_feed(
    cache: &mut ControllerCache,
    configs: &OracleProviderConfig,
    asset: &Address,
) -> i128 {
    match configs.oracle_type {
        OracleType::Normal => normal_price(cache, configs, asset),
        OracleType::None => panic_with_error!(cache.env(), OracleError::InvalidOracleTokenType),
    }
}

// ---------------------------------------------------------------------------
// Normal token pricing
// ---------------------------------------------------------------------------

fn normal_price(
    cache: &mut ControllerCache,
    configs: &OracleProviderConfig,
    asset: &Address,
) -> i128 {
    let market = cache.cached_market_config(asset);
    let max_stale = configs.max_price_stale_seconds;

    match configs.exchange_source {
        ExchangeSource::SpotOnly => {
            // Dev/test mode: single spot price, no TWAP, no deviation check.
            cex_spot_price(cache, asset, &market, max_stale)
        }
        ExchangeSource::DualOracle => {
            // Production Tier 1: CEX TWAP vs Stellar DEX spot cross-validation.
            // DEX unavailability degrades gracefully to TWAP-only and never
            // blocks the transaction.
            let twap = cex_twap_price(cache, asset, &market, max_stale);
            let dex = dex_spot_price(cache, asset, &market, max_stale);
            calculate_final_price(cache, dex, Some(twap), configs)
        }
        _ => {
            // SpotVsTwap (default): CEX spot as aggregator, CEX TWAP as safe.
            let (spot, twap) = cex_spot_and_twap_price(cache, asset, &market, max_stale);
            calculate_final_price(cache, Some(spot), Some(twap), configs)
        }
    }
}

// ---------------------------------------------------------------------------
// Final price selection with tolerance validation
// ---------------------------------------------------------------------------

pub(crate) fn calculate_final_price(
    cache: &ControllerCache,
    aggregator: Option<i128>,
    safe: Option<i128>,
    configs: &OracleProviderConfig,
) -> i128 {
    let env = cache.env();
    match (aggregator, safe) {
        (Some(agg_price), Some(safe_price)) => {
            let tol = &configs.tolerance;
            if is_within_anchor(
                env,
                agg_price,
                safe_price,
                tol.first_upper_ratio_bps,
                tol.first_lower_ratio_bps,
            ) {
                safe_price
            } else if is_within_anchor(
                env,
                agg_price,
                safe_price,
                tol.last_upper_ratio_bps,
                tol.last_lower_ratio_bps,
            ) {
                agg_price
                    .checked_add(safe_price)
                    .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
                    / 2
            } else {
                // Block risk-increasing ops; allow supply and repay.
                if !cache.allow_unsafe_price {
                    panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
                }
                safe_price
            }
        }
        (Some(agg_price), None) => agg_price,
        (None, Some(safe_price)) => safe_price,
        (None, None) => {
            panic_with_error!(env, OracleError::NoLastPrice);
        }
    }
}

// ---------------------------------------------------------------------------
// Reflector price helpers
// ---------------------------------------------------------------------------

fn to_reflector_asset(
    asset: &Address,
    kind: &ReflectorAssetKind,
    symbol: &soroban_sdk::Symbol,
) -> ReflectorAsset {
    match kind {
        ReflectorAssetKind::Stellar => ReflectorAsset::Stellar(asset.clone()),
        ReflectorAssetKind::Other => ReflectorAsset::Other(symbol.clone()),
    }
}

fn check_staleness(cache: &ControllerCache, feed_ts: u64, max_stale: u64) {
    let now_secs = cache.current_timestamp_ms / 1000;
    let is_stale = now_secs > feed_ts && (now_secs - feed_ts) > max_stale;
    // Staleness is bypassed only when the caller opted into the unsafe-price
    // flag. Permissive caches keep risk-decreasing and view paths live during
    // an oracle outage; strict caches panic.
    if is_stale && !cache.allow_unsafe_price {
        panic_with_error!(cache.env(), OracleError::PriceFeedStale);
    }
    // The clock-skew gate is intentionally unconditional: a future-dated
    // oracle is always malicious or malfunctioning, regardless of the risk
    // direction of the calling op.
    check_not_future(cache, feed_ts);
}

/// Reject oracle timestamps significantly in the future (allow 60s clock skew).
/// Future-dated prices indicate a malicious or malfunctioning oracle feed.
fn check_not_future(cache: &ControllerCache, feed_ts: u64) {
    let now_secs = cache.current_timestamp_ms / 1000;
    let max_future_ts = now_secs
        .checked_add(60)
        .unwrap_or_else(|| panic_with_error!(cache.env(), GenericError::MathOverflow));
    if feed_ts > max_future_ts {
        panic_with_error!(cache.env(), OracleError::PriceFeedStale);
    }
}

fn min_twap_observations(records: u32) -> u32 {
    core::cmp::max(1, records.div_ceil(2))
}

fn cex_spot_price(
    cache: &mut ControllerCache,
    asset: &Address,
    market: &MarketConfig,
    max_stale: u64,
) -> i128 {
    let env = cache.env();
    let cex_oracle = market
        .cex_oracle
        .clone()
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));
    let ra = to_reflector_asset(asset, &market.cex_asset_kind, &market.cex_symbol);

    let pd = reflector_lastprice_call(env, &cex_oracle, &ra)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::NoLastPrice));

    check_staleness(cache, pd.timestamp, max_stale);

    Wad::from_token(pd.price, market.cex_decimals).raw()
}

fn cex_spot_and_twap_price(
    cache: &mut ControllerCache,
    asset: &Address,
    market: &MarketConfig,
    max_stale: u64,
) -> (i128, i128) {
    let env = cache.env();
    let cex_oracle = market
        .cex_oracle
        .clone()
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));
    let ra = to_reflector_asset(asset, &market.cex_asset_kind, &market.cex_symbol);
    let decimals = market.cex_decimals;

    let spot_pd = reflector_lastprice_call(env, &cex_oracle, &ra)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::NoLastPrice));
    check_staleness(cache, spot_pd.timestamp, max_stale);
    let spot_wad = Wad::from_token(spot_pd.price, decimals).raw();

    if market.twap_records == 0 {
        return (spot_wad, spot_wad);
    }

    let history = reflector_prices_call(env, &cex_oracle, &ra, market.twap_records);
    let Some(history) = history else {
        return (spot_wad, spot_wad);
    };
    if history.is_empty() {
        return (spot_wad, spot_wad);
    }

    let mut sum: i128 = 0;
    let mut oldest_ts: u64 = u64::MAX;

    for pd in history.iter() {
        sum = sum
            .checked_add(pd.price)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
        // Track the oldest sample timestamp so the freshness gate reflects
        // the worst input rather than the best.
        if pd.timestamp < oldest_ts {
            oldest_ts = pd.timestamp;
        }
    }

    if history.len() < min_twap_observations(market.twap_records) {
        panic_with_error!(env, OracleError::TwapInsufficientObservations);
    }

    check_staleness(cache, oldest_ts, max_stale);
    let twap_wad = Wad::from_token(sum / history.len() as i128, decimals).raw();

    (spot_wad, twap_wad)
}

fn cex_twap_price(
    cache: &mut ControllerCache,
    asset: &Address,
    market: &MarketConfig,
    max_stale: u64,
) -> i128 {
    if market.twap_records == 0 {
        // TWAP disabled (dev/test); fall back directly to spot.
        return cex_spot_price(cache, asset, market, max_stale);
    }

    let env = cache.env();
    let cex_oracle = market
        .cex_oracle
        .clone()
        .unwrap_or_else(|| panic_with_error!(env, OracleError::OracleNotConfigured));
    let ra = to_reflector_asset(asset, &market.cex_asset_kind, &market.cex_symbol);
    let decimals = market.cex_decimals;

    let history = reflector_prices_call(env, &cex_oracle, &ra, market.twap_records);
    let Some(history) = history else {
        // No history available. Fall back to spot rather than blocking the
        // entire protocol when the TWAP window is empty.
        return cex_spot_price(cache, asset, market, max_stale);
    };
    if history.is_empty() {
        return cex_spot_price(cache, asset, market, max_stale);
    }

    let mut sum: i128 = 0;
    let mut oldest_ts: u64 = u64::MAX;

    for pd in history.iter() {
        sum = sum
            .checked_add(pd.price)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
        // Track the oldest sample so the freshness gate uses the worst input.
        if pd.timestamp < oldest_ts {
            oldest_ts = pd.timestamp;
        }
    }

    if history.len() < min_twap_observations(market.twap_records) {
        panic_with_error!(env, OracleError::TwapInsufficientObservations);
    }

    check_staleness(cache, oldest_ts, max_stale);

    Wad::from_token(sum / history.len() as i128, decimals).raw()
}

fn dex_spot_price(
    cache: &mut ControllerCache,
    asset: &Address,
    market: &MarketConfig,
    max_stale: u64,
) -> Option<i128> {
    let dex_addr = market.dex_oracle.clone()?;

    let env = cache.env();
    let ra = to_reflector_asset(asset, &market.dex_asset_kind, &market.dex_symbol);

    let pd = reflector_lastprice_call(env, &dex_addr, &ra)?; // None: asset not tracked on Stellar DEX oracle.

    // DEX staleness is soft: treat stale as unavailable; allow fallback.
    let now_secs = cache.current_timestamp_ms / 1000;
    if now_secs > pd.timestamp && (now_secs - pd.timestamp) > max_stale {
        return None;
    }

    Some(Wad::from_token(pd.price, market.dex_decimals).raw())
}

// ---------------------------------------------------------------------------
// Tolerance validation
// ---------------------------------------------------------------------------

crate::summarized!(
    is_within_anchor_summary,
    pub(crate) fn is_within_anchor(
        env: &Env,
        aggregator: i128,
        safe: i128,
        upper_bound_ratio: u32,
        lower_bound_ratio: u32,
    ) -> bool {
        if aggregator == 0 {
            return false;
        }
        // Compute ratio: safe / aggregator in RAY precision, then rescale to BPS.
        let ratio_ray = Ray::from_raw(safe)
            .div(env, Ray::from_raw(aggregator))
            .raw();
        let ratio_bps = fp_core::rescale_half_up(ratio_ray, 27, 4); // RAY -> BPS decimals.
        let upper = i128::from(upper_bound_ratio);
        let lower = i128::from(lower_bound_ratio);

        ratio_bps <= upper && ratio_bps >= lower
    }
);

// ---------------------------------------------------------------------------
// Price components (for views / monitoring)
// ---------------------------------------------------------------------------

pub fn price_components(
    cache: &mut ControllerCache,
    asset: &Address,
) -> (Option<i128>, Option<i128>, i128, bool, bool) {
    let market = cache.cached_market_config(asset);
    let configs = market.oracle_config;

    if configs.oracle_type != OracleType::Normal {
        let final_price = token_price(cache, asset).price_wad;
        return (None, None, final_price, true, true);
    }

    let market = cache.cached_market_config(asset);
    let max_stale = configs.max_price_stale_seconds;

    match configs.exchange_source {
        ExchangeSource::SpotOnly => {
            let spot = cex_spot_price(cache, asset, &market, max_stale);
            (Some(spot), None, spot, true, true)
        }
        ExchangeSource::DualOracle => {
            let safe_price = cex_twap_price(cache, asset, &market, max_stale);
            let aggregator_price = dex_spot_price(cache, asset, &market, max_stale);
            let final_price =
                calculate_final_price(cache, aggregator_price, Some(safe_price), &configs);

            match aggregator_price {
                Some(aggregator_price) => {
                    let within_first = is_within_anchor(
                        cache.env(),
                        aggregator_price,
                        safe_price,
                        configs.tolerance.first_upper_ratio_bps,
                        configs.tolerance.first_lower_ratio_bps,
                    );
                    let within_second = within_first
                        || is_within_anchor(
                            cache.env(),
                            aggregator_price,
                            safe_price,
                            configs.tolerance.last_upper_ratio_bps,
                            configs.tolerance.last_lower_ratio_bps,
                        );
                    (
                        Some(aggregator_price),
                        Some(safe_price),
                        final_price,
                        within_first,
                        within_second,
                    )
                }
                None => (None, Some(safe_price), final_price, true, true),
            }
        }
        _ => {
            let (aggregator_price, safe_price) =
                cex_spot_and_twap_price(cache, asset, &market, max_stale);
            let final_price =
                calculate_final_price(cache, Some(aggregator_price), Some(safe_price), &configs);
            let within_first = is_within_anchor(
                cache.env(),
                aggregator_price,
                safe_price,
                configs.tolerance.first_upper_ratio_bps,
                configs.tolerance.first_lower_ratio_bps,
            );
            let within_second = within_first
                || is_within_anchor(
                    cache.env(),
                    aggregator_price,
                    safe_price,
                    configs.tolerance.last_upper_ratio_bps,
                    configs.tolerance.last_lower_ratio_bps,
                );

            (
                Some(aggregator_price),
                Some(safe_price),
                final_price,
                within_first,
                within_second,
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Index update
// ---------------------------------------------------------------------------

crate::summarized!(
    update_asset_index_summary,
    pub fn update_asset_index(cache: &mut ControllerCache, asset: &Address) -> MarketIndex {
        let env = cache.env().clone();
        let sync_data = cache.cached_pool_sync_data(asset);
        simulate_update_indexes(
            &env,
            cache.current_timestamp_ms,
            sync_data.state.last_timestamp,
            Ray::from_raw(sync_data.state.borrowed_ray),
            Ray::from_raw(sync_data.state.borrow_index_ray),
            Ray::from_raw(sync_data.state.supplied_ray),
            Ray::from_raw(sync_data.state.supply_index_ray),
            &sync_data.params,
        )
    }
);

crate::summarized!(
    reflector::lastprice_summary,
    pub(crate) fn reflector_lastprice_call(
        env: &Env,
        oracle: &Address,
        asset: &ReflectorAsset,
    ) -> Option<reflector::ReflectorPriceData> {
        ReflectorClient::new(env, oracle).lastprice(asset)
    }
);

crate::summarized!(
    reflector::prices_summary,
    pub(crate) fn reflector_prices_call(
        env: &Env,
        oracle: &Address,
        asset: &ReflectorAsset,
        records: u32,
    ) -> Option<soroban_sdk::Vec<reflector::ReflectorPriceData>> {
        ReflectorClient::new(env, oracle).prices(asset, &records)
    }
);
