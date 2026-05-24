use common::constants::WAD;
use common::types::{
    MarketOracleConfigInput, OracleAssetRef, OracleReadMode, OracleSourceConfigInput,
    OracleSourceConfigInputOption, OracleStrategy, RedStoneSourceConfigInput,
    ReflectorSourceConfigInput,
};
use soroban_sdk::{Address, String};

// ---------------------------------------------------------------------------
// Price helpers (all return i128, WAD-scaled)
// ---------------------------------------------------------------------------

/// Whole-dollar price: usd(1) = 1 WAD, usd(2000) = 2000 WAD.
pub const fn usd(n: i128) -> i128 {
    n * WAD
}

/// Cent-precision price: usd_cents(50) = $0.50.
pub const fn usd_cents(n: i128) -> i128 {
    n * WAD / 100
}

/// Fractional price: usd_frac(3, 10) = $0.30.
pub const fn usd_frac(num: i128, den: i128) -> i128 {
    num * WAD / den
}

// ---------------------------------------------------------------------------
// Time helpers (all return u64 seconds)
// ---------------------------------------------------------------------------

pub const fn days(n: u64) -> u64 {
    n * 86_400
}

pub const fn hours(n: u64) -> u64 {
    n * 3_600
}

pub const fn minutes(n: u64) -> u64 {
    n * 60
}

pub const fn secs(n: u64) -> u64 {
    n
}

// ---------------------------------------------------------------------------
// Amount helpers
// ---------------------------------------------------------------------------

/// Convert a human-readable amount to on-chain representation.
/// tokens(1000, 7) = 1000_0000000.
pub fn tokens(n: i128, decimals: u32) -> i128 {
    n * 10i128.pow(decimals)
}

/// Identity function -- documents that a value is in basis points.
pub const fn bps(n: i128) -> i128 {
    n
}

/// Convert f64 amount to i128 using asset decimals.
/// f64_to_i128(1000.5, 7) = 10005000000.
pub fn f64_to_i128(amount: f64, decimals: u32) -> i128 {
    (amount * 10f64.powi(decimals as i32)) as i128
}

/// Convert i128 to f64 using asset decimals.
pub fn i128_to_f64(amount: i128, decimals: u32) -> f64 {
    amount as f64 / 10f64.powi(decimals as i32)
}

/// Convert WAD-scaled i128 to f64 (divide by 10^18).
pub fn wad_to_f64(amount: i128) -> f64 {
    amount as f64 / WAD as f64
}

pub const DEFAULT_REDSTONE_MAX_STALE_SECONDS: u64 = 900;
pub const DEFAULT_MIN_SANITY_PRICE_WAD: i128 = 1;
pub const DEFAULT_MAX_SANITY_PRICE_WAD: i128 = common::constants::MAX_REASONABLE_PRICE_WAD;

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
