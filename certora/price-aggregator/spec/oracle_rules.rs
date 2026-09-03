use cvlr::macros::rule;
use cvlr::nondet::nondet;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use price_aggregator_interface::PriceAggregatorInterface;
use soroban_sdk::{panic_with_error, Address, Env, Vec};

use common::constants::{MAX_REASONABLE_PRICE_WAD, MAX_TOLERANCE, MIN_TOLERANCE, WAD};
use common::errors::OracleError;
use common::types::{
    AssetOracle, FeedSource, IndependencePolicy, OracleAssetRef, OracleReadMode, OracleTolerance,
    PriceFeedRaw, PriceKey, PriceSource, ProviderRef, ReflectorFeedRef, ScaledSource,
};

fn midpoint_if_in_band(e: &Env, anchor: i128, primary: i128, tolerance: &OracleTolerance) -> i128 {
    if !crate::tolerance::within_tolerance_band(e, anchor, primary, tolerance) {
        panic_with_error!(e, OracleError::UnsafePriceNotAllowed);
    }
    crate::tolerance::midpoint_price_or_zero(anchor, primary)
}

/// Resolve one key through the public `prices` entrypoint.
///
/// Replaces the former single-key `PriceAggregator::price`, which was removed in
/// favour of the batch endpoint; that function's body was exactly this, so rules
/// written against it keep their meaning — including the revert on missing
/// config, which still originates inside `engine::resolve`.
fn single_price(e: Env, key: PriceKey) -> PriceFeedRaw {
    let requested = soroban_sdk::vec![&e, key.clone()];
    crate::PriceAggregator::prices(e, requested).get_unchecked(key)
}

/// The registry's own price ceiling.
///
/// `validate_sanity_bounds` (`common/src/validation.rs`) rejects any oracle
/// whose `max_sanity_price_wad` exceeds `MAX_REASONABLE_PRICE_WAD` (1e9 WAD),
/// so no accepted feed can report more. The band rules below stay a single
/// multiply-divide at this ceiling: `within_tolerance_band` computes
/// `high * BPS`, at most 1e31, which never leaves the native `i128` path.
const MAX_REALISTIC_PRICE: i128 = MAX_REASONABLE_PRICE_WAD;
const PAR_RATIO_BPS: u32 = 10_000;

fn pinned_oracle(env: &Env, asset: &Address, oracle: Address) -> AssetOracle {
    let mut sources = Vec::new(env);
    sources.push_back(PriceSource::Feed(FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: oracle,
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode: OracleReadMode::Spot,
        }),
        decimals: 14,
        max_stale_seconds: 900,
    }));
    AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_200,
            lower_ratio_bps: 9_800,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: 9 * WAD,
        max_sanity_price_wad: 11 * WAD,
    }
}

/// [`pinned_oracle`] with its single Reflector leg read as a 3-record TWAP
/// instead of `Spot`.
///
/// `validation::smoothing` rejects any configuration whose only source is an
/// unsmoothed market leg (`GenericError::SpotOnlyNotProductionSafe`), and a
/// Reflector reference is `Market` nature and unsmoothed exactly when its read
/// mode is `Spot` (`common/src/types/composable_oracle.rs`). `pinned_oracle` is
/// therefore a shape `validate_asset_oracle` always rejects, whatever its
/// sanity bounds or quote key are. The `*_fixture_completes` witnesses below
/// need a configuration the validator can accept, so they switch the read mode
/// and change nothing else. `Twap(3)` is the smallest record count
/// `feed_shape` admits (`MIN_SMOOTHING_TWAP_RECORDS`).
fn smoothed_oracle(env: &Env, asset: &Address, oracle: Address) -> AssetOracle {
    let mut config = pinned_oracle(env, asset, oracle.clone());
    let mut sources = Vec::new(env);
    sources.push_back(PriceSource::Feed(FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: oracle,
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode: OracleReadMode::Twap(3),
        }),
        decimals: 14,
        max_stale_seconds: 900,
    }));
    config.sources = sources;
    config
}

fn assert_blend_within_inputs(e: &Env, first: i128, second: i128, tolerance_bps: u32) {
    let tolerance = OracleTolerance {
        upper_ratio_bps: PAR_RATIO_BPS + tolerance_bps,
        lower_ratio_bps: PAR_RATIO_BPS - tolerance_bps,
    };
    let final_price = midpoint_if_in_band(e, first, second, &tolerance);
    let min_price = first.min(second);
    let max_price = first.max(second);
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
    let mut session = crate::session::Session::new(&e);
    let key = PriceKey::Token(asset);

    let price_wad: i128 = nondet();
    let asset_decimals: u32 = nondet();
    let timestamp: u64 = nondet();
    cvlr_assume!(price_wad > 0 && price_wad <= MAX_REALISTIC_PRICE);
    cvlr_assume!(asset_decimals <= 27);
    let now_secs = session.now_secs();
    cvlr_assume!(timestamp <= now_secs + 60);
    let seeded = PriceFeedRaw {
        price_wad,
        asset_decimals,
        timestamp,
    };
    session.store_price(&key, seeded.clone());

    let feed = crate::engine::resolve(&mut session, &key, 0);

    cvlr_assert!(feed.price_wad == seeded.price_wad);
    cvlr_assert!(feed.asset_decimals == seeded.asset_decimals);
    cvlr_assert!(feed.timestamp == seeded.timestamp);
}

#[rule]
fn single_price_respects_configured_sanity_bounds(e: Env, asset: Address, oracle: Address) {
    cvlr_assume!(asset != oracle);
    let config = pinned_oracle(&e, &asset, oracle);
    let key = PriceKey::Token(asset.clone());
    crate::registry::store_oracle(&e, &key, &config);

    let feed = single_price(e, key);
    cvlr_assert!(feed.price_wad >= config.min_sanity_price_wad);
    cvlr_assert!(feed.price_wad <= config.max_sanity_price_wad);
    cvlr_assert!(feed.asset_decimals == config.asset_decimals);
}

#[rule]
fn price_endpoint_sanity(e: Env, asset: Address, oracle: Address) {
    cvlr_assume!(asset != oracle);
    let config = pinned_oracle(&e, &asset, oracle);
    let key = PriceKey::Token(asset);
    crate::registry::store_oracle(&e, &key, &config);

    let feed = single_price(e, key);
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
    let k1 = PriceKey::Token(first.clone());
    let k2 = PriceKey::Token(second.clone());
    crate::registry::store_oracle(&e, &k1, &pinned_oracle(&e, &first, oracle.clone()));
    crate::registry::store_oracle(&e, &k2, &pinned_oracle(&e, &second, oracle));

    let requested = soroban_sdk::vec![&e, k1.clone(), k2.clone()];
    let prices = crate::PriceAggregator::prices(e, requested);
    cvlr_assert!(prices.contains_key(k1));
    cvlr_assert!(prices.contains_key(k2));
}

fn nondet_partial_outcome(
    e: &Env,
    config: &AssetOracle,
    reading_wad: i128,
) -> crate::engine::Outcome {
    crate::engine::blend_partial(e, config, reading_wad, nondet(), nondet(), nondet::<bool>())
}

/// An oracle whose sources produced no reading never yields a feed.
///
/// Revert shape. Its satisfy twin is [`price_endpoint_sanity`], not a new rule:
/// that witness stores the same `pinned_oracle` configuration and drives
/// `prices` -> `engine::resolve` -> `compute_hard` -> `force` on a non-empty leg
/// set, which is exactly this fixture with the empty-legs gate flipped. Adding
/// a second witness would need a `blend_two` harness door in `engine.rs` that
/// does not exist, and would prove nothing `price_endpoint_sanity` does not.
#[rule]
fn empty_legs_force_reverts(e: Env, asset: Address, oracle: Address) {
    cvlr_assume!(asset != oracle);
    let config = pinned_oracle(&e, &asset, oracle);
    let outcome = crate::engine::blend_empty(&e, &config);
    let _ = crate::engine::force(&e, &outcome, Some(&config));
    cvlr_assert!(false);
}

/// A two-source oracle with only one leg reading never yields a feed.
///
/// Revert shape. Satisfy twin: [`price_endpoint_sanity`], for the same reason
/// as [`empty_legs_force_reverts`] -- a partial outcome is never usable by
/// construction, so the only completing outcome on this fixture is the one the
/// resolve path builds.
#[rule]
fn partial_legs_force_reverts(e: Env, asset: Address, oracle: Address, reading_wad: i128) {
    cvlr_assume!(asset != oracle);
    cvlr_assume!(reading_wad > 0 && reading_wad <= MAX_REALISTIC_PRICE);
    let config = pinned_oracle(&e, &asset, oracle);
    let outcome = nondet_partial_outcome(&e, &config, reading_wad);
    let _ = crate::engine::force(&e, &outcome, Some(&config));
    cvlr_assert!(false);
}

#[rule]
fn empty_legs_soft_invalid(e: Env, asset: Address, oracle: Address) {
    cvlr_assume!(asset != oracle);
    let config = pinned_oracle(&e, &asset, oracle);
    let outcome = crate::engine::blend_empty(&e, &config);
    let status = crate::engine::to_status(&outcome, Some(&config));
    cvlr_assert!(!status.valid);
}

#[rule]
fn partial_legs_soft_deviation(e: Env, asset: Address, oracle: Address, reading_wad: i128) {
    cvlr_assume!(asset != oracle);
    cvlr_assume!(reading_wad > 0 && reading_wad <= MAX_REALISTIC_PRICE);
    let config = pinned_oracle(&e, &asset, oracle);
    let outcome = nondet_partial_outcome(&e, &config, reading_wad);
    let status = crate::engine::to_status(&outcome, Some(&config));
    cvlr_assert!(status.deviation);
    cvlr_assert!(!status.valid);
}

/// An unregistered key cannot be priced.
///
/// Revert shape. Satisfy twin: [`price_endpoint_sanity`], which is literally
/// this fixture with the gate flipped -- it stores an oracle for the key and
/// then reaches a positive price through the same `single_price` call.
#[rule]
fn missing_oracle_config_reverts(e: Env, asset: Address) {
    let key = PriceKey::Token(asset);
    cvlr_assume!(crate::registry::get_oracle(&e, &key).is_none());
    let _ = single_price(e, key);
    cvlr_assert!(false);
}

/// A scaled source may not quote the key it prices: `properties_of_key` pushes
/// the key onto the session's resolution stack before recursing, so the
/// self-reference is caught as `OracleCycleDetected`.
///
/// Revert shape; paired with `self_quoted_scaled_source_reverts_fixture_completes`.
#[rule]
fn self_quoted_scaled_source_reverts(e: Env, asset: Address, oracle: Address) {
    cvlr_assume!(asset != oracle);
    let mut config = pinned_oracle(&e, &asset, oracle);
    let PriceSource::Feed(feed) = config.sources.get_unchecked(0) else {
        unreachable!()
    };
    let mut sources = Vec::new(&e);
    sources.push_back(PriceSource::Scaled(ScaledSource {
        factor: feed,
        quote: PriceKey::Token(asset.clone()),
        min_factor_wad: WAD,
        max_factor_wad: 2 * WAD,
    }));
    config.sources = sources;

    crate::admin::validate_asset_oracle(&e, &PriceKey::Token(asset), &config);
    cvlr_assert!(false);
}

/// A zero-width, zero-valued sanity band is rejected. `validate_sanity_bounds`
/// runs before every other check in `validate_asset_oracle`, so this is the
/// gate that fires.
///
/// Revert shape; paired with `invalid_sanity_bounds_revert_fixture_completes`.
#[rule]
fn invalid_sanity_bounds_revert(e: Env, asset: Address, oracle: Address) {
    cvlr_assume!(asset != oracle);
    let mut config = pinned_oracle(&e, &asset, oracle);
    config.min_sanity_price_wad = 0;
    config.max_sanity_price_wad = 0;

    crate::admin::validate_asset_oracle(&e, &PriceKey::Token(asset), &config);
    cvlr_assert!(false);
}

/// Satisfy twin of [`self_quoted_scaled_source_reverts`]: the same scaled
/// source, with the self-quote gate flipped by pointing `quote` at a different
/// registered key.
///
/// The quote key needs its own registered oracle, because `properties_of_key`
/// panics with `OracleNotConfigured` on an unregistered dependency; both
/// configurations use [`smoothed_oracle`] so the smoothing check does not fire
/// first and mask the result.
#[rule]
fn self_quoted_scaled_source_reverts_fixture_completes(
    e: Env,
    asset: Address,
    quote_asset: Address,
    oracle: Address,
) {
    cvlr_assume!(asset != oracle && quote_asset != oracle && asset != quote_asset);
    let quote_key = PriceKey::Token(quote_asset.clone());
    crate::registry::store_oracle(
        &e,
        &quote_key,
        &smoothed_oracle(&e, &quote_asset, oracle.clone()),
    );

    let mut config = smoothed_oracle(&e, &asset, oracle);
    let PriceSource::Feed(feed) = config.sources.get_unchecked(0) else {
        unreachable!()
    };
    let mut sources = Vec::new(&e);
    sources.push_back(PriceSource::Scaled(ScaledSource {
        factor: feed,
        quote: quote_key,
        min_factor_wad: WAD,
        max_factor_wad: 2 * WAD,
    }));
    config.sources = sources;

    crate::admin::validate_asset_oracle(&e, &PriceKey::Token(asset), &config);
    cvlr_satisfy!(config.max_sanity_price_wad > config.min_sanity_price_wad);
}

/// Satisfy twin of [`invalid_sanity_bounds_revert`]: the same single-feed
/// configuration with the bounds gate flipped back to the valid
/// `[9 WAD, 11 WAD]` band (band width 1000 bps, exactly
/// `MAX_SINGLE_SOURCE_SANITY_BAND_BPS`).
///
/// This witness is what shows the revert rule is not passing on the smoothing
/// check instead of on the bounds check: it holds every other field fixed and
/// only restores the bounds.
#[rule]
fn invalid_sanity_bounds_revert_fixture_completes(e: Env, asset: Address, oracle: Address) {
    cvlr_assume!(asset != oracle);
    let config = smoothed_oracle(&e, &asset, oracle);

    crate::admin::validate_asset_oracle(&e, &PriceKey::Token(asset), &config);
    cvlr_satisfy!(config.max_sanity_price_wad > config.min_sanity_price_wad);
}
