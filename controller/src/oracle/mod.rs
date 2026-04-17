pub mod reflector;

use common::errors::{GenericError, OracleError};
use common::fp::{Ray, Wad};
use common::fp_core;
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

pub fn token_price(cache: &mut ControllerCache, asset: &Address) -> PriceFeed {
    // Transaction-level cache hit.
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
                (agg_price + safe_price) / 2
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
    if now_secs > feed_ts && (now_secs - feed_ts) > max_stale {
        panic_with_error!(cache.env(), OracleError::PriceFeedStale);
    }
    check_not_future(cache, feed_ts);
}

/// Reject oracle timestamps significantly in the future (allow 60s clock skew).
/// Future-dated prices indicate a malicious or malfunctioning oracle feed.
fn check_not_future(cache: &ControllerCache, feed_ts: u64) {
    let now_secs = cache.current_timestamp_ms / 1000;
    if feed_ts > now_secs.saturating_add(60) {
        panic_with_error!(cache.env(), OracleError::PriceFeedStale);
    }
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
    let client = ReflectorClient::new(env, &cex_oracle);
    let ra = to_reflector_asset(asset, &market.cex_asset_kind, &market.cex_symbol);

    let pd = client
        .lastprice(&ra)
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
    let client = ReflectorClient::new(env, &cex_oracle);
    let ra = to_reflector_asset(asset, &market.cex_asset_kind, &market.cex_symbol);
    let decimals = market.cex_decimals;

    let spot_pd = client
        .lastprice(&ra)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::NoLastPrice));
    check_staleness(cache, spot_pd.timestamp, max_stale);
    let spot_wad = Wad::from_token(spot_pd.price, decimals).raw();

    if market.twap_records == 0 {
        return (spot_wad, spot_wad);
    }

    let history = client.prices(&ra, &market.twap_records);
    let mut sum: i128 = 0;
    let mut count: i128 = 0;
    let mut oldest_ts: u64 = u64::MAX;

    for pd in history.iter().flatten() {
        sum += pd.price;
        count += 1;
        // Track the oldest sample timestamp so the freshness gate reflects
        // the worst input rather than the best.
        if pd.timestamp < oldest_ts {
            oldest_ts = pd.timestamp;
        }
    }

    if count == 0 {
        return (spot_wad, spot_wad);
    }

    let min_required = core::cmp::max(1, (market.twap_records as i128) / 2);
    if count < min_required {
        panic_with_error!(env, OracleError::TwapInsufficientObservations);
    }

    check_staleness(cache, oldest_ts, max_stale);
    let twap_wad = Wad::from_token(sum / count, decimals).raw();

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
    let client = ReflectorClient::new(env, &cex_oracle);
    let ra = to_reflector_asset(asset, &market.cex_asset_kind, &market.cex_symbol);
    let decimals = market.cex_decimals;

    let history = client.prices(&ra, &market.twap_records);

    let mut sum: i128 = 0;
    let mut count: i128 = 0;
    let mut oldest_ts: u64 = u64::MAX;

    for pd in history.iter().flatten() {
        sum += pd.price;
        count += 1;
        // Track the oldest sample so the freshness gate uses the worst input.
        if pd.timestamp < oldest_ts {
            oldest_ts = pd.timestamp;
        }
        // `None` slots mark oracle gaps in the window; accept partial TWAP
        // rather than panic.
    }

    if count == 0 {
        // All N slots were None, indicating a major oracle outage. Fall
        // back to the spot price rather than blocking the entire protocol.
        return cex_spot_price(cache, asset, market, max_stale);
    }

    let min_required = core::cmp::max(1, (market.twap_records as i128) / 2);
    if count < min_required {
        panic_with_error!(env, OracleError::TwapInsufficientObservations);
    }

    check_staleness(cache, oldest_ts, max_stale);

    Wad::from_token(sum / count, decimals).raw()
}

fn dex_spot_price(
    cache: &mut ControllerCache,
    asset: &Address,
    market: &MarketConfig,
    max_stale: u64,
) -> Option<i128> {
    let dex_addr = market.dex_oracle.clone()?;

    let env = cache.env();
    let client = ReflectorClient::new(env, &dex_addr);
    let ra = to_reflector_asset(asset, &market.dex_asset_kind, &market.dex_symbol);

    let pd = client.lastprice(&ra)?; // None: asset not tracked on Stellar DEX oracle.

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

pub(crate) fn is_within_anchor(
    env: &Env,
    aggregator: i128,
    safe: i128,
    upper_bound_ratio: i128,
    lower_bound_ratio: i128,
) -> bool {
    if aggregator == 0 {
        return false;
    }
    // Compute ratio: safe / aggregator in RAY precision, then rescale to BPS.
    let ratio_ray = common::fp::Ray::from_raw(safe)
        .div(env, common::fp::Ray::from_raw(aggregator))
        .raw();
    let ratio_bps = fp_core::rescale_half_up(ratio_ray, 27, 4); // RAY -> BPS decimals.

    ratio_bps <= upper_bound_ratio && ratio_bps >= lower_bound_ratio
}

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

pub fn update_asset_index(
    cache: &mut ControllerCache,
    asset: &Address,
    simulate: bool,
) -> MarketIndex {
    let env = cache.env().clone();

    if simulate {
        let pool_addr = cache.cached_pool_address(asset);
        let pool_client = pool_interface::LiquidityPoolClient::new(&env, &pool_addr);
        let sync_data = pool_client.get_sync_data();
        common::rates::simulate_update_indexes(
            &env,
            cache.current_timestamp_ms,
            sync_data.state.last_timestamp,
            Ray::from_raw(sync_data.state.borrowed_ray),
            Ray::from_raw(sync_data.state.borrow_index_ray),
            Ray::from_raw(sync_data.state.supplied_ray),
            Ray::from_raw(sync_data.state.supply_index_ray),
            &sync_data.params,
        )
    } else {
        let _feed = token_price(cache, asset); // Mutating path: refresh price and sync state.
        let pool_addr = cache.cached_pool_address(asset);
        let pool_client = pool_interface::LiquidityPoolClient::new(&env, &pool_addr);
        pool_client.update_indexes(&0)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::types::{
        AssetConfig, ExchangeSource, MarketConfig, MarketStatus, OraclePriceFluctuation,
        OracleType, ReflectorConfig,
    };
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::{contract, contractimpl, contracttype, Address, Symbol, Vec};

    #[contracttype]
    #[derive(Clone)]
    enum MockKey {
        Spot(reflector::ReflectorAsset),
        History(reflector::ReflectorAsset),
    }

    #[contract]
    struct MockReflector;

    #[contractimpl]
    impl MockReflector {
        pub fn set_spot(env: Env, asset: reflector::ReflectorAsset, price: i128, timestamp: u64) {
            env.storage().temporary().set(
                &MockKey::Spot(asset),
                &reflector::ReflectorPriceData { price, timestamp },
            );
        }

        pub fn set_history(
            env: Env,
            asset: reflector::ReflectorAsset,
            history: Vec<Option<reflector::ReflectorPriceData>>,
        ) {
            env.storage()
                .temporary()
                .set(&MockKey::History(asset), &history);
        }

        pub fn decimals(_env: Env) -> u32 {
            14
        }

        pub fn resolution(_env: Env) -> u32 {
            300
        }

        pub fn lastprice(
            env: Env,
            asset: reflector::ReflectorAsset,
        ) -> Option<reflector::ReflectorPriceData> {
            env.storage().temporary().get(&MockKey::Spot(asset))
        }

        pub fn prices(
            env: Env,
            asset: reflector::ReflectorAsset,
            records: u32,
        ) -> Vec<Option<reflector::ReflectorPriceData>> {
            if let Some(history) = env
                .storage()
                .temporary()
                .get(&MockKey::History(asset.clone()))
            {
                return history;
            }

            let mut out = Vec::new(&env);
            let spot = Self::lastprice(env.clone(), asset);
            for _ in 0..records {
                out.push_back(spot.clone());
            }
            out
        }
    }

    struct TestSetup {
        env: Env,
        controller: Address,
        asset: Address,
        pool: Address,
        cex_oracle: Address,
        dex_oracle: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set(LedgerInfo {
                timestamp: 1_000,
                protocol_version: 25,
                sequence_number: 100,
                network_id: Default::default(),
                base_reserve: 10,
                min_temp_entry_ttl: 10,
                min_persistent_entry_ttl: 10,
                max_entry_ttl: 3_110_400,
            });

            let admin = Address::generate(&env);
            let controller = env.register(crate::Controller, (admin.clone(),));
            let asset = env
                .register_stellar_asset_contract_v2(admin.clone())
                .address()
                .clone();
            let pool = Address::generate(&env);

            let cex_oracle = env.register(MockReflector, ());
            let dex_oracle = env.register(MockReflector, ());

            Self {
                env,
                controller,
                asset,
                pool,
                cex_oracle,
                dex_oracle,
            }
        }

        fn as_controller<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.controller, f)
        }

        fn tolerance(&self) -> OraclePriceFluctuation {
            OraclePriceFluctuation {
                first_upper_ratio_bps: 10_200,
                first_lower_ratio_bps: 9_800,
                last_upper_ratio_bps: 11_000,
                last_lower_ratio_bps: 9_000,
            }
        }

        fn market_config(
            &self,
            oracle_type: OracleType,
            exchange_source: ExchangeSource,
        ) -> MarketConfig {
            MarketConfig {
                status: if oracle_type == OracleType::None {
                    MarketStatus::PendingOracle
                } else {
                    MarketStatus::Active
                },
                asset_config: AssetConfig {
                    loan_to_value_bps: 7_500,
                    liquidation_threshold_bps: 8_000,
                    liquidation_bonus_bps: 500,
                    liquidation_fees_bps: 100,
                    is_collateralizable: true,
                    is_borrowable: true,
                    e_mode_enabled: false,
                    is_isolated_asset: false,
                    is_siloed_borrowing: false,
                    is_flashloanable: true,
                    isolation_borrow_enabled: true,
                    isolation_debt_ceiling_usd_wad: 1_000_000,
                    flashloan_fee_bps: 9,
                    borrow_cap: 2_000_000,
                    supply_cap: 3_000_000,
                },
                pool_address: self.pool.clone(),
                oracle_config: OracleProviderConfig {
                    base_asset: self.asset.clone(),
                    oracle_type,
                    exchange_source,
                    asset_decimals: 7,
                    tolerance: self.tolerance(),
                    max_price_stale_seconds: 900,
                },
                cex_oracle: None,
                cex_asset_kind: ReflectorAssetKind::Stellar,
                cex_symbol: Symbol::new(&self.env, ""),
                cex_decimals: 0,
                dex_oracle: None,
                dex_asset_kind: ReflectorAssetKind::Stellar,
                dex_symbol: Symbol::new(&self.env, ""),
                dex_decimals: 0,
                twap_records: 0,
            }
        }

        fn reflector_config(
            &self,
            cex_kind: ReflectorAssetKind,
            dex_oracle: Option<Address>,
        ) -> ReflectorConfig {
            ReflectorConfig {
                cex_oracle: self.cex_oracle.clone(),
                cex_asset_kind: cex_kind,
                cex_symbol: Symbol::new(&self.env, "XLM"),
                cex_decimals: 14,
                dex_oracle,
                dex_asset_kind: ReflectorAssetKind::Stellar,
                dex_decimals: 14,
                twap_records: 3,
            }
        }

        fn market_with_reflector(
            &self,
            oracle_type: OracleType,
            exchange_source: ExchangeSource,
            reflector: ReflectorConfig,
        ) -> MarketConfig {
            let mut market = self.market_config(oracle_type, exchange_source);
            market.cex_oracle = Some(reflector.cex_oracle);
            market.cex_asset_kind = reflector.cex_asset_kind;
            market.cex_symbol = reflector.cex_symbol.clone();
            market.cex_decimals = reflector.cex_decimals;
            market.dex_oracle = reflector.dex_oracle;
            market.dex_asset_kind = reflector.dex_asset_kind;
            market.dex_symbol = reflector.cex_symbol;
            market.dex_decimals = reflector.dex_decimals;
            market.twap_records = reflector.twap_records;
            market
        }

        fn configure_market(
            &self,
            oracle_type: OracleType,
            exchange_source: ExchangeSource,
            reflector: Option<ReflectorConfig>,
        ) {
            self.as_controller(|| {
                crate::storage::set_market_config(
                    &self.env,
                    &self.asset,
                    &self.market_config(oracle_type, exchange_source),
                );
                if let Some(reflector) = reflector {
                    crate::storage::set_reflector_config(&self.env, &self.asset, &reflector);
                }
            });
        }
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #12)")]
    fn test_token_price_rejects_pending_oracle_market() {
        let t = TestSetup::new();

        t.configure_market(OracleType::None, ExchangeSource::SpotOnly, None);
        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let _ = token_price(&mut cache, &t.asset);
        });
    }

    #[test]
    fn test_token_price_allows_disabled_market_for_opted_in_flows() {
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        t.configure_market(
            OracleType::Normal,
            ExchangeSource::SpotOnly,
            Some(t.reflector_config(ReflectorAssetKind::Stellar, None)),
        );
        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &123_000_000,
            &1_000, // match ledger timestamp -- stale prices are no longer accepted
        );

        t.as_controller(|| {
            let mut market = crate::storage::get_market_config(&t.env, &t.asset);
            market.status = MarketStatus::Disabled;
            crate::storage::set_market_config(&t.env, &t.asset, &market);

            let mut cache = ControllerCache::new_with_disabled_market_price(&t.env, true);
            let feed = token_price(&mut cache, &t.asset);
            assert_eq!(feed.asset_decimals, 7);
            assert!(feed.price_wad > 0);
        });
    }

    #[test]
    fn test_calculate_final_price_and_anchor_bounds() {
        let t = TestSetup::new();

        t.as_controller(|| {
            let safe_cache = ControllerCache::new(&t.env, true);
            let config = t.market_config(OracleType::Normal, ExchangeSource::SpotOnly);

            assert_eq!(
                calculate_final_price(&safe_cache, Some(100), Some(101), &config.oracle_config),
                101
            );
            assert_eq!(
                calculate_final_price(&safe_cache, Some(100), Some(109), &config.oracle_config),
                104
            );
            assert_eq!(
                calculate_final_price(&safe_cache, Some(100), None, &config.oracle_config),
                100
            );
            assert_eq!(
                calculate_final_price(&safe_cache, None, Some(109), &config.oracle_config),
                109
            );
            assert!(!is_within_anchor(&t.env, 0, 100, 10_200, 9_800));
            assert!(is_within_anchor(&t.env, 100, 102, 10_200, 9_800));
            let reflector_asset = to_reflector_asset(
                &t.asset,
                &ReflectorAssetKind::Other,
                &Symbol::new(&t.env, "ETH"),
            );
            if let reflector::ReflectorAsset::Other(symbol) = reflector_asset {
                assert_eq!(symbol, Symbol::new(&t.env, "ETH"));
            }
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #205)")]
    fn test_calculate_final_price_blocks_unsafe_deviation_for_risk_increasing_ops() {
        let t = TestSetup::new();

        t.as_controller(|| {
            let cache = ControllerCache::new(&t.env, false);
            let config = t.market_config(OracleType::Normal, ExchangeSource::SpotOnly);

            let _ = calculate_final_price(&cache, Some(100), Some(150), &config.oracle_config);
        });
    }

    #[test]
    fn test_calculate_final_price_allows_unsafe_deviation_for_risk_decreasing_ops() {
        let t = TestSetup::new();

        t.as_controller(|| {
            let cache = ControllerCache::new(&t.env, true);
            let config = t.market_config(OracleType::Normal, ExchangeSource::SpotOnly);

            assert_eq!(
                calculate_final_price(&cache, Some(100), Some(150), &config.oracle_config),
                150
            );
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #210)")]
    fn test_calculate_final_price_panics_without_any_sources() {
        let t = TestSetup::new();

        t.as_controller(|| {
            let cache = ControllerCache::new(&t.env, true);
            let config = t.market_config(OracleType::Normal, ExchangeSource::SpotOnly);

            let _ = calculate_final_price(&cache, None, None, &config.oracle_config);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #206)")]
    fn test_check_staleness_panics_even_when_unsafe_price_allowed() {
        let t = TestSetup::new();

        t.as_controller(|| {
            let cache = ControllerCache::new(&t.env, true);
            check_staleness(&cache, 1, 1);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #206)")]
    fn test_check_staleness_blocks_risk_increasing_ops() {
        let t = TestSetup::new();

        t.as_controller(|| {
            let cache = ControllerCache::new(&t.env, false);
            check_staleness(&cache, 1, 1);
        });
    }

    #[test]
    fn test_cex_twap_and_dex_paths_with_mock_reflector() {
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);
        let dex_client = MockReflectorClient::new(&t.env, &t.dex_oracle);

        t.configure_market(
            OracleType::Normal,
            ExchangeSource::DualOracle,
            Some(t.reflector_config(ReflectorAssetKind::Stellar, Some(t.dex_oracle.clone()))),
        );

        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &200_000_000_000_000,
            &1_000,
        );

        let mut partial_history = Vec::new(&t.env);
        partial_history.push_back(Some(reflector::ReflectorPriceData {
            price: 100_000_000_000_000,
            timestamp: 980,
        }));
        partial_history.push_back(None);
        partial_history.push_back(Some(reflector::ReflectorPriceData {
            price: 300_000_000_000_000,
            timestamp: 1_000,
        }));
        cex_client.set_history(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &partial_history,
        );
        dex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &210_000_000_000_000,
            &1_000,
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let rcfg = t.reflector_config(ReflectorAssetKind::Stellar, Some(t.dex_oracle.clone()));
            let market = t.market_with_reflector(
                OracleType::Normal,
                ExchangeSource::DualOracle,
                rcfg.clone(),
            );

            assert_eq!(
                cex_twap_price(&mut cache, &t.asset, &market, 900),
                2_000_000_000_000_000_000
            );
            assert_eq!(
                dex_spot_price(&mut cache, &t.asset, &market, 900),
                Some(2_100_000_000_000_000_000)
            );

            let disabled_twap = ReflectorConfig {
                twap_records: 0,
                ..rcfg.clone()
            };
            let disabled_market = t.market_with_reflector(
                OracleType::Normal,
                ExchangeSource::DualOracle,
                disabled_twap,
            );
            assert_eq!(
                cex_twap_price(&mut cache, &t.asset, &disabled_market, 900),
                2_000_000_000_000_000_000
            );
        });
    }

    #[test]
    fn test_cex_twap_falls_back_to_spot_when_history_is_all_none() {
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        t.configure_market(
            OracleType::Normal,
            ExchangeSource::SpotOnly,
            Some(t.reflector_config(ReflectorAssetKind::Stellar, None)),
        );

        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &400_000_000_000_000,
            &1_000,
        );
        let mut empty_history = Vec::new(&t.env);
        empty_history.push_back(None);
        empty_history.push_back(None);
        cex_client.set_history(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &empty_history,
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let rcfg = t.reflector_config(ReflectorAssetKind::Stellar, None);
            let market =
                t.market_with_reflector(OracleType::Normal, ExchangeSource::SpotOnly, rcfg);

            assert_eq!(
                cex_twap_price(&mut cache, &t.asset, &market, 900),
                4_000_000_000_000_000_000
            );
        });
    }

    #[test]
    fn test_mock_reflector_resolution_and_prices_default_to_spot() {
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &500_000_000_000_000,
            &1_000,
        );

        let history = cex_client.prices(&reflector::ReflectorAsset::Stellar(t.asset.clone()), &2);

        assert_eq!(cex_client.resolution(), 300);
        assert_eq!(history.len(), 2);
        assert_eq!(history.get(0).unwrap().unwrap().price, 500_000_000_000_000);
        assert_eq!(history.get(1).unwrap().unwrap().timestamp, 1_000);
    }

    #[test]
    fn test_dex_spot_price_returns_none_for_disabled_missing_and_stale_inputs() {
        let t = TestSetup::new();
        let dex_client = MockReflectorClient::new(&t.env, &t.dex_oracle);

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let disabled = t.market_with_reflector(
                OracleType::Normal,
                ExchangeSource::DualOracle,
                t.reflector_config(ReflectorAssetKind::Stellar, None),
            );
            assert_eq!(dex_spot_price(&mut cache, &t.asset, &disabled, 900), None);

            let enabled = t.market_with_reflector(
                OracleType::Normal,
                ExchangeSource::DualOracle,
                t.reflector_config(ReflectorAssetKind::Stellar, Some(t.dex_oracle.clone())),
            );
            assert_eq!(dex_spot_price(&mut cache, &t.asset, &enabled, 900), None);
        });

        dex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &123_000_000_000_000,
            &1_000,
        );
        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let enabled = t.market_with_reflector(
                OracleType::Normal,
                ExchangeSource::DualOracle,
                t.reflector_config(ReflectorAssetKind::Stellar, Some(t.dex_oracle.clone())),
            );
            assert_eq!(
                dex_spot_price(&mut cache, &t.asset, &enabled, 900),
                Some(1_230_000_000_000_000_000)
            );
        });

        dex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &123_000_000_000_000,
            &1,
        );
        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let enabled = t.market_with_reflector(
                OracleType::Normal,
                ExchangeSource::DualOracle,
                t.reflector_config(ReflectorAssetKind::Stellar, Some(t.dex_oracle.clone())),
            );
            assert_eq!(dex_spot_price(&mut cache, &t.asset, &enabled, 900), None);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #210)")]
    fn test_cex_spot_price_panics_without_last_price() {
        let t = TestSetup::new();

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let rcfg = t.reflector_config(ReflectorAssetKind::Stellar, None);
            let market =
                t.market_with_reflector(OracleType::Normal, ExchangeSource::SpotOnly, rcfg);
            let _ = cex_spot_price(&mut cache, &t.asset, &market, 900);
        });
    }

    #[test]
    fn test_find_price_feed_dispatches_normal_spot_only_and_dual_oracle_modes() {
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);
        let dex_client = MockReflectorClient::new(&t.env, &t.dex_oracle);

        t.configure_market(
            OracleType::Normal,
            ExchangeSource::SpotOnly,
            Some(t.reflector_config(ReflectorAssetKind::Stellar, None)),
        );
        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &200_000_000_000_000,
            &1_000,
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let config = t.market_config(OracleType::Normal, ExchangeSource::SpotOnly);
            assert_eq!(
                find_price_feed(&mut cache, &config.oracle_config, &t.asset),
                2_000_000_000_000_000_000
            );
        });

        let mut history = Vec::new(&t.env);
        history.push_back(Some(reflector::ReflectorPriceData {
            price: 200_000_000_000_000,
            timestamp: 1_000,
        }));
        history.push_back(Some(reflector::ReflectorPriceData {
            price: 200_000_000_000_000,
            timestamp: 995,
        }));
        cex_client.set_history(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &history,
        );
        dex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &202_000_000_000_000,
            &1_000,
        );
        t.configure_market(
            OracleType::Normal,
            ExchangeSource::DualOracle,
            Some(t.reflector_config(ReflectorAssetKind::Stellar, Some(t.dex_oracle.clone()))),
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let config = t.market_config(OracleType::Normal, ExchangeSource::DualOracle);
            assert_eq!(
                find_price_feed(&mut cache, &config.oracle_config, &t.asset),
                2_000_000_000_000_000_000
            );
        });
    }

    // ------------------------------------------------------------------
    // Additional coverage tests
    // ------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "Error(Contract, #217)")]
    fn test_token_price_rejects_zero_price() {
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        t.configure_market(
            OracleType::Normal,
            ExchangeSource::SpotOnly,
            Some(t.reflector_config(ReflectorAssetKind::Stellar, None)),
        );
        // Zero spot price -> InvalidPrice panic (line 43)
        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &0,
            &1_000,
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let _ = token_price(&mut cache, &t.asset);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #206)")]
    fn test_token_price_rejects_future_timestamp() {
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        t.configure_market(
            OracleType::Normal,
            ExchangeSource::SpotOnly,
            Some(t.reflector_config(ReflectorAssetKind::Stellar, None)),
        );
        // timestamp more than 60s in the future (ledger is 1000) -> PriceFeedStale (line 177)
        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &100_000_000_000_000,
            &5_000,
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let _ = token_price(&mut cache, &t.asset);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #204)")]
    fn test_find_price_feed_panics_for_oracle_type_none() {
        // Covers line 66: OracleType::None branch inside find_price_feed
        let t = TestSetup::new();

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let mut cfg = t
                .market_config(OracleType::Normal, ExchangeSource::SpotOnly)
                .oracle_config;
            cfg.oracle_type = OracleType::None;
            let _ = find_price_feed(&mut cache, &cfg, &t.asset);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #12)")]
    fn test_token_price_pending_oracle_with_oracle_type_none_branch() {
        // Also hits line 38 transitively by configuring OracleType::None and Active status,
        // bypassing the PendingOracle status check. We directly set market config to
        // Active status, with oracle_type None, to drive the `oracle_type == None` check
        // at line 37 inside token_price.
        let t = TestSetup::new();

        t.as_controller(|| {
            // Bypass the status match: set Active status but OracleType::None.
            let mut market = t.market_config(OracleType::Normal, ExchangeSource::SpotOnly);
            market.status = MarketStatus::Active;
            market.oracle_config.oracle_type = OracleType::None;
            crate::storage::set_market_config(&t.env, &t.asset, &market);

            let mut cache = ControllerCache::new(&t.env, true);
            let _ = token_price(&mut cache, &t.asset);
        });
    }

    #[test]
    fn test_cex_spot_and_twap_twap_records_zero_returns_spot() {
        // Covers line 226: early return when twap_records == 0
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &150_000_000_000_000,
            &1_000,
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let mut rcfg = t.reflector_config(ReflectorAssetKind::Stellar, None);
            rcfg.twap_records = 0;
            let market =
                t.market_with_reflector(OracleType::Normal, ExchangeSource::SpotOnly, rcfg);
            let (spot, twap) = cex_spot_and_twap_price(&mut cache, &t.asset, &market, 900);
            assert_eq!(spot, 1_500_000_000_000_000_000);
            assert_eq!(twap, 1_500_000_000_000_000_000);
        });
    }

    #[test]
    fn test_cex_spot_and_twap_count_zero_falls_back_to_spot() {
        // Covers line 243: count == 0 -> return (spot, spot)
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &150_000_000_000_000,
            &1_000,
        );
        let mut all_none = Vec::new(&t.env);
        all_none.push_back(None);
        all_none.push_back(None);
        all_none.push_back(None);
        cex_client.set_history(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &all_none,
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let rcfg = t.reflector_config(ReflectorAssetKind::Stellar, None);
            let market =
                t.market_with_reflector(OracleType::Normal, ExchangeSource::SpotOnly, rcfg);
            let (spot, twap) = cex_spot_and_twap_price(&mut cache, &t.asset, &market, 900);
            assert_eq!(spot, 1_500_000_000_000_000_000);
            assert_eq!(twap, 1_500_000_000_000_000_000);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #219)")]
    fn test_cex_spot_and_twap_insufficient_observations() {
        // Covers line 248: TwapInsufficientObservations in cex_spot_and_twap_price.
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &150_000_000_000_000,
            &1_000,
        );
        // twap_records = 6, only 1 valid observation (< 3 required)
        let mut history = Vec::new(&t.env);
        history.push_back(Some(reflector::ReflectorPriceData {
            price: 150_000_000_000_000,
            timestamp: 1_000,
        }));
        history.push_back(None);
        history.push_back(None);
        history.push_back(None);
        history.push_back(None);
        history.push_back(None);
        cex_client.set_history(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &history,
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let mut rcfg = t.reflector_config(ReflectorAssetKind::Stellar, None);
            rcfg.twap_records = 6;
            let market =
                t.market_with_reflector(OracleType::Normal, ExchangeSource::SpotVsTwap, rcfg);
            let _ = cex_spot_and_twap_price(&mut cache, &t.asset, &market, 900);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #219)")]
    fn test_cex_twap_only_insufficient_observations() {
        // Covers line 301: TwapInsufficientObservations in cex_twap_price.
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &150_000_000_000_000,
            &1_000,
        );
        // twap_records = 6, only 1 valid observation (< 3 required)
        let mut history = Vec::new(&t.env);
        history.push_back(Some(reflector::ReflectorPriceData {
            price: 150_000_000_000_000,
            timestamp: 1_000,
        }));
        history.push_back(None);
        history.push_back(None);
        history.push_back(None);
        history.push_back(None);
        history.push_back(None);
        cex_client.set_history(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &history,
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let mut rcfg = t.reflector_config(ReflectorAssetKind::Stellar, None);
            rcfg.twap_records = 6;
            let market =
                t.market_with_reflector(OracleType::Normal, ExchangeSource::DualOracle, rcfg);
            let _ = cex_twap_price(&mut cache, &t.asset, &market, 900);
        });
    }

    #[test]
    fn test_price_components_for_dual_oracle_within_first_tier() {
        // Covers lines 378-410 (DualOracle branch of price_components, agg=Some).
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);
        let dex_client = MockReflectorClient::new(&t.env, &t.dex_oracle);

        t.configure_market(
            OracleType::Normal,
            ExchangeSource::DualOracle,
            Some(t.reflector_config(ReflectorAssetKind::Stellar, Some(t.dex_oracle.clone()))),
        );
        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &200_000_000_000_000,
            &1_000,
        );
        let mut hist = Vec::new(&t.env);
        hist.push_back(Some(reflector::ReflectorPriceData {
            price: 200_000_000_000_000,
            timestamp: 1_000,
        }));
        hist.push_back(Some(reflector::ReflectorPriceData {
            price: 200_000_000_000_000,
            timestamp: 995,
        }));
        cex_client.set_history(&reflector::ReflectorAsset::Stellar(t.asset.clone()), &hist);
        // Aggregator within first tier (1% deviation)
        dex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &202_000_000_000_000,
            &1_000,
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let (agg, safe, final_price, within_first, within_second) =
                price_components(&mut cache, &t.asset);
            assert!(agg.is_some());
            assert!(safe.is_some());
            assert!(final_price > 0);
            assert!(within_first);
            assert!(within_second);
            // Safe price returned (within first tier)
            assert_eq!(final_price, safe.unwrap());
        });
    }

    #[test]
    fn test_price_components_for_dual_oracle_aggregator_missing() {
        // Covers line 409: None aggregator branch returning (None, Some, final, true, true).
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        t.configure_market(
            OracleType::Normal,
            ExchangeSource::DualOracle,
            // No DEX oracle -> dex_spot_price returns None
            Some(t.reflector_config(ReflectorAssetKind::Stellar, None)),
        );
        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &200_000_000_000_000,
            &1_000,
        );
        let mut hist = Vec::new(&t.env);
        hist.push_back(Some(reflector::ReflectorPriceData {
            price: 200_000_000_000_000,
            timestamp: 1_000,
        }));
        hist.push_back(Some(reflector::ReflectorPriceData {
            price: 200_000_000_000_000,
            timestamp: 995,
        }));
        cex_client.set_history(&reflector::ReflectorAsset::Stellar(t.asset.clone()), &hist);

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let (agg, safe, final_price, within_first, within_second) =
                price_components(&mut cache, &t.asset);
            assert!(agg.is_none());
            assert_eq!(safe, Some(2_000_000_000_000_000_000));
            assert_eq!(final_price, 2_000_000_000_000_000_000);
            assert!(within_first);
            assert!(within_second);
        });
    }

    #[test]
    fn test_token_price_dual_oracle_aggregator_missing_falls_back_to_safe() {
        // Covers the (None, Some) branch in calculate_final_price via DualOracle.
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        t.configure_market(
            OracleType::Normal,
            ExchangeSource::DualOracle,
            Some(t.reflector_config(ReflectorAssetKind::Stellar, None)),
        );
        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &200_000_000_000_000,
            &1_000,
        );
        let mut hist = Vec::new(&t.env);
        hist.push_back(Some(reflector::ReflectorPriceData {
            price: 200_000_000_000_000,
            timestamp: 1_000,
        }));
        hist.push_back(Some(reflector::ReflectorPriceData {
            price: 200_000_000_000_000,
            timestamp: 995,
        }));
        cex_client.set_history(&reflector::ReflectorAsset::Stellar(t.asset.clone()), &hist);

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, false);
            let feed = token_price(&mut cache, &t.asset);
            assert_eq!(feed.price_wad, 2_000_000_000_000_000_000);
        });
    }

    #[test]
    fn test_price_components_non_normal_oracle_type() {
        // Covers lines 366-367: early-return for oracle_type != Normal.
        // We can't easily reach token_price inside this branch (OracleType::None panics),
        // so assert the panic propagates, which still exercises line 366's check.
        let t = TestSetup::new();

        t.as_controller(|| {
            let mut market = t.market_config(OracleType::Normal, ExchangeSource::SpotOnly);
            market.status = MarketStatus::Active;
            market.oracle_config.oracle_type = OracleType::None;
            crate::storage::set_market_config(&t.env, &t.asset, &market);

            let mut cache = ControllerCache::new(&t.env, true);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                price_components(&mut cache, &t.asset)
            }));
            assert!(result.is_err());
        });
    }

    #[test]
    fn test_price_components_for_spot_only() {
        // Covers lines 374-376: SpotOnly branch of price_components.
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        t.configure_market(
            OracleType::Normal,
            ExchangeSource::SpotOnly,
            Some(t.reflector_config(ReflectorAssetKind::Stellar, None)),
        );
        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &200_000_000_000_000,
            &1_000,
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let (agg, safe, final_price, within_first, within_second) =
                price_components(&mut cache, &t.asset);
            assert_eq!(agg, Some(2_000_000_000_000_000_000));
            assert_eq!(safe, None);
            assert_eq!(final_price, 2_000_000_000_000_000_000);
            assert!(within_first);
            assert!(within_second);
        });
    }

    #[test]
    fn test_price_components_for_spot_vs_twap() {
        // Covers lines 412-440 (SpotVsTwap/_ branch with within_second).
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);

        t.configure_market(
            OracleType::Normal,
            ExchangeSource::SpotVsTwap,
            Some(t.reflector_config(ReflectorAssetKind::Stellar, None)),
        );
        // Set spot deliberately deviating ~4% from TWAP (outside 1st tier, inside 2nd tier)
        // Second tier is 10% (9000/11000 BPS). Spot 208, TWAP 200 -> 4% deviation.
        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &208_000_000_000_000,
            &1_000,
        );
        let mut hist = Vec::new(&t.env);
        hist.push_back(Some(reflector::ReflectorPriceData {
            price: 200_000_000_000_000,
            timestamp: 1_000,
        }));
        hist.push_back(Some(reflector::ReflectorPriceData {
            price: 200_000_000_000_000,
            timestamp: 995,
        }));
        cex_client.set_history(&reflector::ReflectorAsset::Stellar(t.asset.clone()), &hist);

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let (agg, safe, _final_price, within_first, within_second) =
                price_components(&mut cache, &t.asset);
            assert_eq!(agg, Some(2_080_000_000_000_000_000));
            assert_eq!(safe, Some(2_000_000_000_000_000_000));
            // 4% deviation: outside first tier (2%), inside second tier (10%).
            assert!(!within_first);
            assert!(within_second);
        });
    }

    #[test]
    fn test_price_components_dual_oracle_within_second_tier() {
        // Covers lines 394-399: DualOracle branch where aggregator exists but
        // falls outside first tier -- forces within_second recomputation.
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);
        let dex_client = MockReflectorClient::new(&t.env, &t.dex_oracle);

        t.configure_market(
            OracleType::Normal,
            ExchangeSource::DualOracle,
            Some(t.reflector_config(ReflectorAssetKind::Stellar, Some(t.dex_oracle.clone()))),
        );
        cex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &200_000_000_000_000,
            &1_000,
        );
        let mut hist = Vec::new(&t.env);
        hist.push_back(Some(reflector::ReflectorPriceData {
            price: 200_000_000_000_000,
            timestamp: 1_000,
        }));
        hist.push_back(Some(reflector::ReflectorPriceData {
            price: 200_000_000_000_000,
            timestamp: 995,
        }));
        cex_client.set_history(&reflector::ReflectorAsset::Stellar(t.asset.clone()), &hist);
        // DEX aggregator deviating ~4% from TWAP (outside first 2%, inside second 10%)
        dex_client.set_spot(
            &reflector::ReflectorAsset::Stellar(t.asset.clone()),
            &208_000_000_000_000,
            &1_000,
        );

        t.as_controller(|| {
            let mut cache = ControllerCache::new(&t.env, true);
            let (agg, safe, _final_price, within_first, within_second) =
                price_components(&mut cache, &t.asset);
            assert!(agg.is_some());
            assert!(safe.is_some());
            assert!(!within_first);
            assert!(within_second);
        });
    }

    #[test]
    fn test_mock_reflector_decimals_helper() {
        // Covers lines 518-520: MockReflector::decimals helper.
        let t = TestSetup::new();
        let cex_client = MockReflectorClient::new(&t.env, &t.cex_oracle);
        assert_eq!(cex_client.decimals(), 14);
    }
}
