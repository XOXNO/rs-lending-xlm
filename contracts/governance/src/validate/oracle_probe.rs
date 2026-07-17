//! Live oracle validation for market configuration.

use common::errors::{GenericError, OracleError};
use common::oracle::observation::{
    millis_to_seconds, normalize_positive_price, u256_to_i128, validate_positive_price_timestamps,
    MIN_ORACLE_RESOLUTION_SECONDS,
};
use common::oracle::providers::redstone::{
    read_price_data_uncached, RedStonePriceData, REDSTONE_DECIMALS,
};
use common::oracle::providers::reflector::{
    min_twap_observations, reflector_base_call, reflector_decimals_call, reflector_lastprice_call,
    reflector_prices_call, reflector_resolution_call, to_reflector_asset, twap_mean_price,
    ReflectorAsset, ReflectorPriceData,
};
use soroban_sdk::Vec;
use common::types::{
    MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation, OracleReadMode,
    OracleSourceConfig, OracleSourceConfigInput, OracleSourceConfigOption, RedStoneSourceConfig,
    RedStoneSourceConfigInput, ReflectorBase, ReflectorSourceConfig,
};
use common::validation::validate_single_source_sanity_band;

use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::validate::asset::validate_and_fetch_token_decimals;
use crate::validate::oracle_config::{
    validate_decimals, validate_max_stale, validate_oracle_config_shape, validate_sanity_bounds,
    validate_twap_records,
};

pub(crate) fn validate_market_oracle_sources(
    env: &Env,
    asset: &Address,
    config: &MarketOracleConfigInput,
    tolerance: OraclePriceFluctuation,
) -> MarketOracleConfig {
    validate_oracle_config_shape(env, config);
    validate_max_stale(env, config.max_price_stale_seconds);
    validate_sanity_bounds(
        env,
        config.min_sanity_price_wad,
        config.max_sanity_price_wad,
    );
    validate_single_source_sanity_band(
        env,
        config.strategy,
        config.min_sanity_price_wad,
        config.max_sanity_price_wad,
    );

    let asset_decimals = validate_and_fetch_token_decimals(env, asset);
    let (primary, primary_usd_wad) =
        validate_source(env, asset, &config.primary, config.max_price_stale_seconds);
    let (anchor, anchor_usd_wad) = match config.anchor.as_ref() {
        Some(anchor) => {
            let (source, price) =
                validate_source(env, asset, anchor, config.max_price_stale_seconds);
            (OracleSourceConfigOption::Some(source), price)
        }
        None => (OracleSourceConfigOption::None, None),
    };

    // Containment probe at PROPOSE, while the feed is fresh: a sanity band that
    // excludes the current live price stores fine but bricks every later risk
    // read (`SanityBoundViolated`) for the asset — borrow, withdraw, liquidation.
    // Reject it here instead. Each USD-denominated leg must resolve inside the
    // band; the composed price is a midpoint of the legs, so both-in-band implies
    // the blend is in-band too. Quoted legs price in their quote asset (USD only
    // after the controller's quote multiply), so they carry no USD price to check
    // here and are covered by the read-time gate. This mirrors the containment
    // that `set_oracle_sanity_bounds` enforces on the immediate band-move path.
    require_price_in_band(env, primary_usd_wad, config);
    require_price_in_band(env, anchor_usd_wad, config);

    MarketOracleConfig {
        asset_decimals,
        max_price_stale_seconds: config.max_price_stale_seconds,
        tolerance,
        strategy: config.strategy,
        primary,
        anchor,
        min_sanity_price_wad: config.min_sanity_price_wad,
        max_sanity_price_wad: config.max_sanity_price_wad,
    }
}

/// Reverts `SanityBoundViolated` when a resolved USD leg price sits outside the
/// config's sanity band. `None` (a quote-denominated leg) carries no USD price
/// to check here.
fn require_price_in_band(env: &Env, price_usd_wad: Option<i128>, config: &MarketOracleConfigInput) {
    if let Some(price) = price_usd_wad {
        assert_with_error!(
            env,
            price >= config.min_sanity_price_wad && price <= config.max_sanity_price_wad,
            OracleError::SanityBoundViolated
        );
    }
}

/// one is directly available. A quote-denominated Reflector leg returns `None`:
/// its live price is in the quote asset, USD only after the controller's
/// quote multiply, so there is no USD price to sanity-check at propose time.
fn validate_source(
    env: &Env,
    asset: &Address,
    source: &OracleSourceConfigInput,
    max_stale: u64,
) -> (OracleSourceConfig, Option<i128>) {
    match source {
        OracleSourceConfigInput::Reflector(config) => {
            let base = validate_base(env, asset, &config.contract);
            let reflector_asset = to_reflector_asset(env, &config.asset);
            let decimals = reflector_decimals_call(env, &config.contract);
            validate_decimals(env, decimals);
            let resolution = reflector_resolution_call(env, &config.contract);
            if resolution < MIN_ORACLE_RESOLUTION_SECONDS || u64::from(resolution) > max_stale {
                panic_with_error!(env, OracleError::InvalidOracleResolution);
            }

            let pd = reflector_lastprice_call(env, &config.contract, &reflector_asset)
                .unwrap_or_else(|| panic_with_error!(env, GenericError::InvalidTicker));
            let spot_wad = validate_reflector_feed(env, &pd, max_stale, decimals);

            // The band-check price must match what the controller composes at
            // read time: the TWAP mean for a Twap source, the spot otherwise.
            // Both sides derive the mean through the shared `twap_mean_price`.
            let read_price_wad = match config.read_mode {
                OracleReadMode::Spot => spot_wad,
                OracleReadMode::Twap(records) => {
                    validate_twap_records(env, records);
                    let history = validate_twap_history(
                        env,
                        &config.contract,
                        &reflector_asset,
                        records,
                        max_stale,
                        decimals,
                    );
                    normalize_positive_price(env, twap_mean_price(env, &history), decimals)
                }
            };

            // Only a USD-based leg carries a USD price here; a quoted leg's price
            // is in its quote asset until the controller resolves the quote hop.
            let usd_price = match base {
                ReflectorBase::Usd => Some(read_price_wad),
                ReflectorBase::Quoted(_) => None,
            };

            (
                OracleSourceConfig::Reflector(ReflectorSourceConfig {
                    contract: config.contract.clone(),
                    asset: config.asset.clone(),
                    read_mode: config.read_mode,
                    decimals,
                    resolution_seconds: resolution,
                    base,
                }),
                usd_price,
            )
        }
        OracleSourceConfigInput::RedStone(config) => {
            // RedStone feeds are 8-decimal USD; the adapter has no `decimals()`.
            let (redstone, price_wad) = validate_feed_id_source(env, config, REDSTONE_DECIMALS);
            (OracleSourceConfig::RedStone(redstone), Some(price_wad))
        }
        OracleSourceConfigInput::Xoxno(config) => {
            let decimals = reflector_decimals_call(env, &config.contract);
            let (xoxno, price_wad) = validate_feed_id_source(env, config, decimals);
            (OracleSourceConfig::Xoxno(xoxno), Some(price_wad))
        }
    }
}

fn validate_feed_id_source(
    env: &Env,
    config: &RedStoneSourceConfigInput,
    decimals: u32,
) -> (RedStoneSourceConfig, i128) {
    validate_max_stale(env, config.max_stale_seconds);
    validate_decimals(env, decimals);

    let Some(price_data) = read_price_data_uncached(env, &config.contract, &config.feed_id) else {
        panic_with_error!(env, GenericError::InvalidTicker);
    };
    let price_wad = validate_redstone_feed(env, &price_data, config.max_stale_seconds, decimals);

    (
        RedStoneSourceConfig {
            contract: config.contract.clone(),
            feed_id: config.feed_id.clone(),
            decimals,
            max_stale_seconds: config.max_stale_seconds,
        },
        price_wad,
    )
}

/// Resolves Reflector base; controller re-checks quote activation.
fn validate_base(env: &Env, asset: &Address, oracle: &Address) -> ReflectorBase {
    match reflector_base_call(env, oracle) {
        ReflectorAsset::Other(symbol) if symbol == soroban_sdk::Symbol::new(env, "USD") => {
            ReflectorBase::Usd
        }
        ReflectorAsset::Stellar(quote) => {
            // Reject self-quotes to avoid recursive price reads.
            assert_with_error!(env, &quote != asset, OracleError::InvalidOracleBase);
            ReflectorBase::Quoted(quote)
        }
        ReflectorAsset::Other(_) => panic_with_error!(env, OracleError::InvalidOracleBase),
    }
}

fn validate_twap_history(
    env: &Env,
    oracle: &Address,
    asset: &ReflectorAsset,
    records: u32,
    max_stale: u64,
    decimals: u32,
) -> Vec<ReflectorPriceData> {
    let history = reflector_prices_call(env, oracle, asset, records)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::ReflectorHistoryEmpty));
    assert_with_error!(env, !history.is_empty(), OracleError::ReflectorHistoryEmpty);
    assert_with_error!(
        env,
        history.len() >= min_twap_observations(records),
        OracleError::TwapInsufficientObservations
    );
    for pd in history.iter() {
        validate_reflector_feed(env, &pd, max_stale, decimals);
    }
    history
}

/// Freshness/positivity check; returns the feed's normalized USD price (WAD).
fn validate_reflector_feed(
    env: &Env,
    pd: &ReflectorPriceData,
    max_stale: u64,
    decimals: u32,
) -> i128 {
    let now = env.ledger().timestamp();
    validate_positive_price_timestamps(
        env,
        pd.price,
        decimals,
        now,
        &[pd.timestamp],
        max_stale,
    )
}

/// Freshness/positivity check; returns the feed's normalized USD price (WAD).
fn validate_redstone_feed(
    env: &Env,
    pd: &RedStonePriceData,
    max_stale: u64,
    decimals: u32,
) -> i128 {
    let raw_price = u256_to_i128(env, &pd.price);
    let now = env.ledger().timestamp();
    validate_positive_price_timestamps(
        env,
        raw_price,
        decimals,
        now,
        &[
            millis_to_seconds(pd.package_timestamp),
            millis_to_seconds(pd.write_timestamp),
        ],
        max_stale,
    )
}
