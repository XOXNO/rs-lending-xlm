//! Market oracle builders for test setup.
//!
//! Builders produce the [`AssetOracle`] shapes the price-aggregator stores.
//! Where a test drives them through governance they face the full validation
//! set, so the defaults here are chosen to be *valid* configurations rather
//! than merely well-typed ones: real decimals for each mock, staleness ceilings
//! that cover every component, and a source mix that satisfies the smoothing
//! and independence rules.

use controller::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleAssetRef,
    OracleReadMode, OracleTolerance, PriceKey, PriceSource, ProviderKind, ProviderRef,
    ReflectorFeedRef, ScaledSource,
};
use soroban_sdk::{Address, Env, String, Vec};

pub const DEFAULT_REDSTONE_MAX_STALE_SECONDS: u64 = 900;
pub const DEFAULT_MIN_SANITY_PRICE_WAD: i128 = 1;
pub const DEFAULT_MAX_SANITY_PRICE_WAD: i128 = controller::constants::MAX_REASONABLE_PRICE_WAD;

/// The mock Reflector publishes at 14 decimals so the WAD rescale is exercised.
const REFLECTOR_DECIMALS: u32 = 14;
/// The mock RedStone adapter publishes at 8 decimals.
const MULTI_FEED_DECIMALS: u32 = 8;

/// Default asset-level staleness ceiling. Every component bound must sit under
/// it, which `validate_staleness_envelope` enforces.
const DEFAULT_MAX_PRICE_STALE_SECONDS: u64 = 900;

pub fn reflector_source(
    oracle: &Address,
    asset: &Address,
    read_mode: OracleReadMode,
) -> PriceSource {
    PriceSource::Feed(FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: oracle.clone(),
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode,
        }),
        decimals: REFLECTOR_DECIMALS,
        max_stale_seconds: DEFAULT_MAX_PRICE_STALE_SECONDS,
    })
}

pub fn redstone_source(contract: &Address, feed_id: &String) -> PriceSource {
    redstone_source_with_max_stale(contract, feed_id, DEFAULT_REDSTONE_MAX_STALE_SECONDS)
}

pub fn redstone_source_with_max_stale(
    contract: &Address,
    feed_id: &String,
    max_stale_seconds: u64,
) -> PriceSource {
    multi_feed_source(contract, feed_id, ProviderKind::RedStone, max_stale_seconds)
}

pub fn xoxno_source(contract: &Address, feed_id: &String) -> PriceSource {
    xoxno_source_with_decimals(contract, feed_id, MULTI_FEED_DECIMALS)
}

/// XOXNO adapters publish their own `decimals()`, so the config declares a width
/// and configure-time attestation matches it against the adapter. Tests that
/// pair a non-default adapter width with a config must pass both.
pub fn xoxno_source_with_decimals(
    contract: &Address,
    feed_id: &String,
    decimals: u32,
) -> PriceSource {
    let mut source = multi_feed_source(
        contract,
        feed_id,
        ProviderKind::Xoxno,
        DEFAULT_REDSTONE_MAX_STALE_SECONDS,
    );
    if let PriceSource::Feed(feed) = &mut source {
        feed.decimals = decimals;
    }
    source
}

/// Push-oracle feeds are declared [`FeedNature::Fundamental`] so a single-source
/// market satisfies the smoothing rule the way the RWA markets do in production:
/// trading cannot move a published NAV, so it needs no window.
fn multi_feed_source(
    contract: &Address,
    feed_id: &String,
    kind: ProviderKind,
    max_stale_seconds: u64,
) -> PriceSource {
    PriceSource::Feed(FeedSource {
        provider: ProviderRef::MultiFeed(MultiFeedRef {
            contract: contract.clone(),
            feed_id: feed_id.clone(),
            kind,
            nature: FeedNature::Fundamental,
        }),
        decimals: MULTI_FEED_DECIMALS,
        max_stale_seconds,
    })
}

/// Reciprocal agreement band, matching what governance computes from BPS:
/// `[BPS^2 / (BPS + t), BPS + t]`. Reciprocal rather than additive so the check
/// is invariant to which source lands in the numerator.
pub fn tolerance_band(env: &Env, tolerance_bps: u32) -> OracleTolerance {
    let bps = controller::constants::BPS;
    let upper = bps + i128::from(tolerance_bps);
    let lower = common::math::fp_core::mul_div_half_up(env, bps, bps, upper);
    OracleTolerance {
        upper_ratio_bps: upper as u32,
        lower_ratio_bps: lower as u32,
    }
}

/// A `Scaled` source: a ratio feed multiplied by the price of `quote`.
///
/// This is the composable model's replacement for v1's implicit Reflector quote
/// hop, and the shape SolvBTC needs: a `SolvBTC/BTC` ratio from one operator
/// times a `BTC/USD` price from another. The ratio leg is a Reflector feed here
/// because the mock speaks that ABI; production ratio feeds are typically
/// multi-feed adapters.
pub fn scaled_reflector_source(
    oracle: &Address,
    asset: &Address,
    quote: PriceKey,
    read_mode: OracleReadMode,
    min_factor_wad: i128,
    max_factor_wad: i128,
) -> PriceSource {
    PriceSource::Scaled(ScaledSource {
        factor: FeedSource {
            provider: ProviderRef::Reflector(ReflectorFeedRef {
                contract: oracle.clone(),
                asset: OracleAssetRef::Stellar(asset.clone()),
                read_mode,
            }),
            decimals: REFLECTOR_DECIMALS,
            max_stale_seconds: DEFAULT_MAX_PRICE_STALE_SECONDS,
        },
        quote,
        min_factor_wad,
        max_factor_wad,
    })
}

/// Single smoothed `Scaled` source with a tight band around `price_wad`.
pub fn scaled_single_config(
    env: &Env,
    oracle_address: &Address,
    asset: &Address,
    quote: PriceKey,
    price_wad: i128,
    tolerance_bps: u32,
) -> AssetOracle {
    let (min_wad, max_wad) = tight_single_source_band(price_wad);
    oracle(
        env,
        &[scaled_reflector_source(
            oracle_address,
            asset,
            quote,
            OracleReadMode::Twap(3),
            1,
            DEFAULT_MAX_SANITY_PRICE_WAD,
        )],
        tolerance_bps,
        min_wad,
        max_wad,
    )
}

/// `Scaled` primary plus a RedStone USD anchor — the dual-source shape where
/// the quote conversion has to happen *before* the agreement band, since one
/// leg is quoted and the other is not.
pub fn scaled_primary_redstone_anchor_config(
    env: &Env,
    reflector_oracle: &Address,
    asset: &Address,
    quote: PriceKey,
    redstone_contract: &Address,
    feed_id: &String,
    tolerance_bps: u32,
) -> AssetOracle {
    oracle(
        env,
        &[
            scaled_reflector_source(
                reflector_oracle,
                asset,
                quote,
                OracleReadMode::Twap(3),
                1,
                DEFAULT_MAX_SANITY_PRICE_WAD,
            ),
            redstone_source(redstone_contract, feed_id),
        ],
        tolerance_bps,
        DEFAULT_MIN_SANITY_PRICE_WAD,
        DEFAULT_MAX_SANITY_PRICE_WAD,
    )
}

/// Rewrites the Reflector read mode on `sources[index]` in place.
///
/// Tests use this to seed a stored config into a shape governance would have
/// rejected (`Twap(0)`, `Twap(MAX+1)`), which is how the read-path fail-closed
/// branches get exercised — validation alone cannot reach them.
///
/// # Panics
/// If that slot is not a direct Reflector feed; a silent no-op would let the
/// test assert the wrong thing and still pass.
pub fn set_reflector_read_mode(oracle: &mut AssetOracle, index: u32, read_mode: OracleReadMode) {
    let PriceSource::Feed(mut feed) = oracle.sources.get_unchecked(index) else {
        panic!("sources[{index}] is not a direct feed");
    };
    let ProviderRef::Reflector(mut reflector) = feed.provider else {
        panic!("sources[{index}] is not a Reflector feed");
    };
    reflector.read_mode = read_mode;
    feed.provider = ProviderRef::Reflector(reflector);
    oracle.sources.set(index, PriceSource::Feed(feed));
}

fn sources(env: &Env, items: &[PriceSource]) -> Vec<PriceSource> {
    let mut out = Vec::new(env);
    for item in items {
        out.push_back(item.clone());
    }
    out
}

/// Highest component bound in `sources`, so the asset ceiling never sits under
/// a leg it is meant to cover.
fn stale_ceiling(sources: &Vec<PriceSource>) -> u64 {
    let mut ceiling = DEFAULT_MAX_PRICE_STALE_SECONDS;
    for source in sources.iter() {
        if let PriceSource::Feed(feed) = &source {
            if feed.max_stale_seconds > ceiling {
                ceiling = feed.max_stale_seconds;
            }
        }
    }
    ceiling
}

fn oracle(
    env: &Env,
    items: &[PriceSource],
    tolerance_bps: u32,
    min_sanity_price_wad: i128,
    max_sanity_price_wad: i128,
) -> AssetOracle {
    let sources = sources(env, items);
    AssetOracle {
        // Overwritten from the token by the governance resolver; harness paths
        // that seed directly do not depend on it.
        asset_decimals: 7,
        max_price_stale_seconds: stale_ceiling(&sources),
        sources,
        tolerance: tolerance_band(env, tolerance_bps),
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad,
        max_sanity_price_wad,
    }
}

pub fn reflector_primary_anchor_config(
    env: &Env,
    oracle_address: &Address,
    asset: &Address,
    price_wad: i128,
    tolerance_bps: u32,
) -> AssetOracle {
    // A same-provider pair is not independent, so the harness default is a
    // single smoothed source with a tight band around the seeded price.
    let (min_wad, max_wad) = tight_single_source_band(price_wad);
    oracle(
        env,
        &[reflector_source(
            oracle_address,
            asset,
            OracleReadMode::Twap(3),
        )],
        tolerance_bps,
        min_wad,
        max_wad,
    )
}

/// ±1% sanity band around `price_wad`, comfortably inside the protocol's
/// `MAX_SINGLE_SOURCE_SANITY_BAND_BPS` (10%) cap for single-source markets.
/// The shared wide default spans the whole `MAX_REASONABLE_PRICE_WAD` domain
/// and only fits dual-source builders, which the cap does not apply to.
///
/// Public so harness `set_price` can re-center the live band when tests move a
/// price, without weakening the production fail-closed checks.
pub fn tight_single_source_band(price_wad: i128) -> (i128, i128) {
    (price_wad - price_wad / 100, price_wad + price_wad / 100)
}

/// A lone spot Reflector source.
///
/// Governance refuses this shape with `SpotOnlyNotProductionSafe` (#38) unless
/// some earlier check fires first — one unsmoothed market leg and nothing to
/// check it against is exactly what the smoothing rule exists to stop. Use it
/// for seeded read-path tests, or where the assertion is about a check that
/// runs *before* smoothing (the base-asset probe, for instance). For a valid
/// single-source listing use [`reflector_primary_anchor_config`], which smooths.
pub fn reflector_single_spot_config(
    env: &Env,
    oracle_address: &Address,
    asset: &Address,
    price_wad: i128,
    tolerance_bps: u32,
) -> AssetOracle {
    let (min_wad, max_wad) = tight_single_source_band(price_wad);
    oracle(
        env,
        &[reflector_source(
            oracle_address,
            asset,
            OracleReadMode::Spot,
        )],
        tolerance_bps,
        min_wad,
        max_wad,
    )
}

pub fn redstone_single_config(
    env: &Env,
    contract: &Address,
    feed_id: &String,
    price_wad: i128,
    tolerance_bps: u32,
) -> AssetOracle {
    let (min_wad, max_wad) = tight_single_source_band(price_wad);
    oracle(
        env,
        &[redstone_source(contract, feed_id)],
        tolerance_bps,
        min_wad,
        max_wad,
    )
}

pub fn xoxno_single_config(
    env: &Env,
    contract: &Address,
    feed_id: &String,
    price_wad: i128,
    tolerance_bps: u32,
) -> AssetOracle {
    xoxno_single_config_with_decimals(
        env,
        contract,
        feed_id,
        MULTI_FEED_DECIMALS,
        price_wad,
        tolerance_bps,
    )
}

pub fn xoxno_single_config_with_decimals(
    env: &Env,
    contract: &Address,
    feed_id: &String,
    decimals: u32,
    price_wad: i128,
    tolerance_bps: u32,
) -> AssetOracle {
    let (min_wad, max_wad) = tight_single_source_band(price_wad);
    oracle(
        env,
        &[xoxno_source_with_decimals(contract, feed_id, decimals)],
        tolerance_bps,
        min_wad,
        max_wad,
    )
}

pub fn reflector_primary_xoxno_anchor_config(
    env: &Env,
    reflector_oracle: &Address,
    asset: &Address,
    xoxno_contract: &Address,
    feed_id: &String,
    tolerance_bps: u32,
) -> AssetOracle {
    oracle(
        env,
        &[
            reflector_source(reflector_oracle, asset, OracleReadMode::Twap(3)),
            xoxno_source(xoxno_contract, feed_id),
        ],
        tolerance_bps,
        DEFAULT_MIN_SANITY_PRICE_WAD,
        DEFAULT_MAX_SANITY_PRICE_WAD,
    )
}

pub fn reflector_primary_redstone_anchor_config(
    env: &Env,
    reflector_oracle: &Address,
    asset: &Address,
    redstone_contract: &Address,
    feed_id: &String,
    tolerance_bps: u32,
) -> AssetOracle {
    oracle(
        env,
        &[
            reflector_source(reflector_oracle, asset, OracleReadMode::Twap(3)),
            redstone_source(redstone_contract, feed_id),
        ],
        tolerance_bps,
        DEFAULT_MIN_SANITY_PRICE_WAD,
        DEFAULT_MAX_SANITY_PRICE_WAD,
    )
}

pub fn reflector_primary_redstone_anchor_config_with_anchor_stale(
    env: &Env,
    reflector_oracle: &Address,
    asset: &Address,
    redstone_contract: &Address,
    feed_id: &String,
    redstone_max_stale_seconds: u64,
    tolerance_bps: u32,
) -> AssetOracle {
    oracle(
        env,
        &[
            reflector_source(reflector_oracle, asset, OracleReadMode::Twap(3)),
            redstone_source_with_max_stale(redstone_contract, feed_id, redstone_max_stale_seconds),
        ],
        tolerance_bps,
        DEFAULT_MIN_SANITY_PRICE_WAD,
        DEFAULT_MAX_SANITY_PRICE_WAD,
    )
}
