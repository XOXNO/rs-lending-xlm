//! Market oracle configuration builders for test setup (`configure_market_oracle`).

use controller::types::{
    MarketOracleConfigInput, OracleAssetRef, OracleReadMode, OracleSourceConfigInput,
    OracleSourceConfigInputOption, OracleStrategy, RedStoneSourceConfigInput,
    ReflectorSourceConfigInput,
};
use soroban_sdk::{Address, String};

pub const DEFAULT_REDSTONE_MAX_STALE_SECONDS: u64 = 900;
pub const DEFAULT_MIN_SANITY_PRICE_WAD: i128 = 1;
pub const DEFAULT_MAX_SANITY_PRICE_WAD: i128 = controller::constants::MAX_REASONABLE_PRICE_WAD;

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
