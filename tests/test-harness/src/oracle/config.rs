use controller::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleAssetRef,
    OracleReadMode, OracleTolerance, PriceKey, PriceSource, ProviderRef, ReflectorFeedRef,
    ScaledSource,
};
use soroban_sdk::{Address, Env, String, Vec};

pub const DEFAULT_REDSTONE_MAX_STALE_SECONDS: u64 = 900;
pub const DEFAULT_MIN_SANITY_PRICE_WAD: i128 = 1;
pub const DEFAULT_MAX_SANITY_PRICE_WAD: i128 = controller::constants::MAX_REASONABLE_PRICE_WAD;

const REFLECTOR_DECIMALS: u32 = 14;

const MULTI_FEED_DECIMALS: u32 = 8;

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
    multi_feed_source(
        ProviderRef::RedStone(multi_feed_ref(contract, feed_id)),
        max_stale_seconds,
    )
}

pub fn xoxno_source(contract: &Address, feed_id: &String) -> PriceSource {
    xoxno_source_with_decimals(contract, feed_id, MULTI_FEED_DECIMALS)
}

pub fn xoxno_source_with_decimals(
    contract: &Address,
    feed_id: &String,
    decimals: u32,
) -> PriceSource {
    let mut source = multi_feed_source(
        ProviderRef::Xoxno(multi_feed_ref(contract, feed_id)),
        DEFAULT_REDSTONE_MAX_STALE_SECONDS,
    );
    if let PriceSource::Feed(feed) = &mut source {
        feed.decimals = decimals;
    }
    source
}

fn multi_feed_source(provider: ProviderRef, max_stale_seconds: u64) -> PriceSource {
    PriceSource::Feed(FeedSource {
        provider,
        decimals: MULTI_FEED_DECIMALS,
        max_stale_seconds,
    })
}

fn multi_feed_ref(contract: &Address, feed_id: &String) -> MultiFeedRef {
    MultiFeedRef {
        contract: contract.clone(),
        feed_id: feed_id.clone(),
        nature: FeedNature::Fundamental,
    }
}

pub fn tolerance_band(env: &Env, tolerance_bps: u32) -> OracleTolerance {
    let bps = controller::constants::BPS;
    let upper = bps + i128::from(tolerance_bps);
    let lower = common::math::fp_core::mul_div_half_up(env, bps, bps, upper);
    OracleTolerance {
        upper_ratio_bps: upper as u32,
        lower_ratio_bps: lower as u32,
    }
}

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

pub fn tight_single_source_band(price_wad: i128) -> (i128, i128) {
    (price_wad - price_wad / 100, price_wad + price_wad / 100)
}

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
