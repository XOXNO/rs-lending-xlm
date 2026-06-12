//! Market oracle configuration builders and resolvers for test setup.
//!
//! Builders produce `MarketOracleConfigInput` shapes; `resolve_market_oracle_config`
//! probes the mock oracles to compute the resolved `MarketOracleConfig` that the
//! governance contract derives in production, ready for the controller's thin
//! `set_market_oracle_config` setter.

use common::oracle::providers::redstone::REDSTONE_DECIMALS;
use controller::constants::BPS;
use controller::types::{
    MarketOracleConfig, MarketOracleConfigInput, OracleAssetRef, OraclePriceFluctuation,
    OracleReadMode, OracleSourceConfig, OracleSourceConfigInput, OracleSourceConfigInputOption,
    OracleSourceConfigOption, OracleStrategy, RedStoneSourceConfig, RedStoneSourceConfigInput,
    ReflectorBase, ReflectorSourceConfig, ReflectorSourceConfigInput,
};
use soroban_sdk::{token, Address, Env, String, Symbol};

use crate::mock_reflector::{MockReflectorClient, Sep40Asset};

pub const DEFAULT_REDSTONE_MAX_STALE_SECONDS: u64 = 900;
pub const DEFAULT_MIN_SANITY_PRICE_WAD: i128 = 1;
pub const DEFAULT_MAX_SANITY_PRICE_WAD: i128 = controller::constants::MAX_REASONABLE_PRICE_WAD;

/// Builds the four `OraclePriceFluctuation` ratio bands from first/last
/// tolerance inputs in BPS: `upper = BPS + tolerance`,
/// `lower = BPS * BPS / upper` (half-up).
pub fn tolerance_bands(
    env: &Env,
    first_tolerance_bps: u32,
    last_tolerance_bps: u32,
) -> OraclePriceFluctuation {
    let (first_upper, first_lower) = tolerance_range(env, first_tolerance_bps);
    let (last_upper, last_lower) = tolerance_range(env, last_tolerance_bps);
    OraclePriceFluctuation {
        first_upper_ratio_bps: first_upper,
        first_lower_ratio_bps: first_lower,
        last_upper_ratio_bps: last_upper,
        last_lower_ratio_bps: last_lower,
    }
}

fn tolerance_range(env: &Env, tolerance_bps: u32) -> (u32, u32) {
    let upper = BPS + i128::from(tolerance_bps);
    let lower = common::math::fp_core::mul_div_half_up(env, BPS, BPS, upper);
    (upper as u32, lower as u32)
}

/// Resolves a config input into the `MarketOracleConfig` the deleted on-chain
/// validation produced: asset decimals from the token, Reflector
/// decimals/resolution/base from live mock reads, RedStone decimals fixed at
/// `REDSTONE_DECIMALS`, tolerance via `tolerance_bands`.
pub fn resolve_market_oracle_config(
    env: &Env,
    asset: &Address,
    input: &MarketOracleConfigInput,
) -> MarketOracleConfig {
    let asset_decimals = token::Client::new(env, asset).decimals();
    let anchor = match input.anchor.as_ref() {
        Some(source) => OracleSourceConfigOption::Some(resolve_source(env, source)),
        None => OracleSourceConfigOption::None,
    };
    MarketOracleConfig {
        asset_decimals,
        max_price_stale_seconds: input.max_price_stale_seconds,
        tolerance: tolerance_bands(env, input.first_tolerance_bps, input.last_tolerance_bps),
        strategy: input.strategy,
        primary: resolve_source(env, &input.primary),
        anchor,
        min_sanity_price_wad: input.min_sanity_price_wad,
        max_sanity_price_wad: input.max_sanity_price_wad,
    }
}

fn resolve_source(env: &Env, source: &OracleSourceConfigInput) -> OracleSourceConfig {
    match source {
        OracleSourceConfigInput::Reflector(config) => {
            let reflector = MockReflectorClient::new(env, &config.contract);
            let base = match reflector.base() {
                Sep40Asset::Other(symbol) if symbol == Symbol::new(env, "USD") => {
                    ReflectorBase::Usd
                }
                Sep40Asset::Stellar(quote) => ReflectorBase::Quoted(quote),
                other => panic!("unsupported mock reflector base: {:?}", other),
            };
            OracleSourceConfig::Reflector(ReflectorSourceConfig {
                contract: config.contract.clone(),
                asset: config.asset.clone(),
                read_mode: config.read_mode,
                decimals: reflector.decimals(),
                resolution_seconds: reflector.resolution(),
                base,
            })
        }
        OracleSourceConfigInput::RedStone(config) => {
            OracleSourceConfig::RedStone(RedStoneSourceConfig {
                contract: config.contract.clone(),
                feed_id: config.feed_id.clone(),
                decimals: REDSTONE_DECIMALS,
                max_stale_seconds: config.max_stale_seconds,
            })
        }
    }
}

pub fn reflector_source(
    oracle: &Address,
    asset: &Address,
    read_mode: OracleReadMode,
) -> OracleSourceConfigInput {
    OracleSourceConfigInput::Reflector(ReflectorSourceConfigInput {
        contract: oracle.clone(),
        asset: OracleAssetRef::Stellar(asset.clone()),
        read_mode,
    })
}

pub fn redstone_source(contract: &Address, feed_id: &String) -> OracleSourceConfigInput {
    redstone_source_with_max_stale(contract, feed_id, DEFAULT_REDSTONE_MAX_STALE_SECONDS)
}

pub fn redstone_source_with_max_stale(
    contract: &Address,
    feed_id: &String,
    max_stale_seconds: u64,
) -> OracleSourceConfigInput {
    OracleSourceConfigInput::RedStone(RedStoneSourceConfigInput {
        contract: contract.clone(),
        feed_id: feed_id.clone(),
        max_stale_seconds,
    })
}

pub fn reflector_primary_anchor_config(
    oracle: &Address,
    asset: &Address,
    first_tolerance_bps: u32,
    last_tolerance_bps: u32,
) -> MarketOracleConfigInput {
    MarketOracleConfigInput {
        max_price_stale_seconds: 900,
        first_tolerance_bps,
        last_tolerance_bps,
        min_sanity_price_wad: DEFAULT_MIN_SANITY_PRICE_WAD,
        max_sanity_price_wad: DEFAULT_MAX_SANITY_PRICE_WAD,
        strategy: OracleStrategy::PrimaryWithAnchor,
        primary: reflector_source(oracle, asset, OracleReadMode::Twap(3)),
        anchor: OracleSourceConfigInputOption::Some(reflector_source(
            oracle,
            asset,
            OracleReadMode::Spot,
        )),
    }
}

pub fn reflector_single_spot_config(
    oracle: &Address,
    asset: &Address,
    first_tolerance_bps: u32,
    last_tolerance_bps: u32,
) -> MarketOracleConfigInput {
    MarketOracleConfigInput {
        max_price_stale_seconds: 900,
        first_tolerance_bps,
        last_tolerance_bps,
        min_sanity_price_wad: DEFAULT_MIN_SANITY_PRICE_WAD,
        max_sanity_price_wad: DEFAULT_MAX_SANITY_PRICE_WAD,
        strategy: OracleStrategy::Single,
        primary: reflector_source(oracle, asset, OracleReadMode::Spot),
        anchor: OracleSourceConfigInputOption::None,
    }
}

pub fn redstone_single_config(
    contract: &Address,
    feed_id: &String,
    first_tolerance_bps: u32,
    last_tolerance_bps: u32,
) -> MarketOracleConfigInput {
    MarketOracleConfigInput {
        max_price_stale_seconds: 900,
        first_tolerance_bps,
        last_tolerance_bps,
        min_sanity_price_wad: DEFAULT_MIN_SANITY_PRICE_WAD,
        max_sanity_price_wad: DEFAULT_MAX_SANITY_PRICE_WAD,
        strategy: OracleStrategy::Single,
        primary: redstone_source(contract, feed_id),
        anchor: OracleSourceConfigInputOption::None,
    }
}

pub fn reflector_primary_redstone_anchor_config(
    reflector_oracle: &Address,
    asset: &Address,
    redstone_contract: &Address,
    feed_id: &String,
    first_tolerance_bps: u32,
    last_tolerance_bps: u32,
) -> MarketOracleConfigInput {
    MarketOracleConfigInput {
        max_price_stale_seconds: 900,
        first_tolerance_bps,
        last_tolerance_bps,
        min_sanity_price_wad: DEFAULT_MIN_SANITY_PRICE_WAD,
        max_sanity_price_wad: DEFAULT_MAX_SANITY_PRICE_WAD,
        strategy: OracleStrategy::PrimaryWithAnchor,
        primary: reflector_source(reflector_oracle, asset, OracleReadMode::Twap(3)),
        anchor: OracleSourceConfigInputOption::Some(redstone_source(redstone_contract, feed_id)),
    }
}

pub fn reflector_primary_redstone_anchor_config_with_anchor_stale(
    reflector_oracle: &Address,
    asset: &Address,
    redstone_contract: &Address,
    feed_id: &String,
    redstone_max_stale_seconds: u64,
    first_tolerance_bps: u32,
    last_tolerance_bps: u32,
) -> MarketOracleConfigInput {
    MarketOracleConfigInput {
        max_price_stale_seconds: 900,
        first_tolerance_bps,
        last_tolerance_bps,
        min_sanity_price_wad: DEFAULT_MIN_SANITY_PRICE_WAD,
        max_sanity_price_wad: DEFAULT_MAX_SANITY_PRICE_WAD,
        strategy: OracleStrategy::PrimaryWithAnchor,
        primary: reflector_source(reflector_oracle, asset, OracleReadMode::Twap(3)),
        anchor: OracleSourceConfigInputOption::Some(redstone_source_with_max_stale(
            redstone_contract,
            feed_id,
            redstone_max_stale_seconds,
        )),
    }
}
