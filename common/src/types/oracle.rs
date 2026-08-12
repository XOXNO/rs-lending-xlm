//! Oracle feed reference and price-result types shared across the price aggregator and its
//! callers: how a feed identifies its underlying asset, the raw and typed forms of a
//! resolved price, and the detailed status returned for dual-source assets.

use soroban_sdk::{contracttype, Address, Env, String, Symbol};

/// Identifies the underlying asset a price feed reports on, in whichever form the source
/// provider expects: a Stellar contract address, a symbol, or a string identifier.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleAssetRef {
    Stellar(Address),

    Symbol(Symbol),

    String(String),
}

/// Basis-point band around a reference price within which a second source's reading is
/// considered non-deviant: `upper_ratio_bps` bounds the allowed excess, `lower_ratio_bps`
/// the allowed shortfall.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleTolerance {
    pub upper_ratio_bps: u32,

    pub lower_ratio_bps: u32,
}

/// How a feed is sampled: an instantaneous spot read, or a multi-observation
/// average over the given number of recorded samples (`Twap` mode name is
/// historical; Reflector implements equal-weight mean, not duration-weighted).
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OracleReadMode {
    Spot,

    Twap(u32),
}

/// Wire form of a resolved price: WAD-scaled price, the asset's token decimals, and the
/// timestamp the price was observed at.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PriceFeedRaw {
    pub price_wad: i128,

    pub asset_decimals: u32,

    pub timestamp: u64,
}

/// Detailed result of resolving an asset's price: the blended final price, the individual
/// primary and secondary leg prices, the timestamp of the older leg, and flags for
/// staleness, cross-source deviation, and overall validity.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceStatus {
    pub final_wad: i128,

    pub primary_wad: i128,

    pub secondary_wad: i128,

    pub price_timestamp: u64,

    pub stale: bool,

    pub deviation: bool,

    pub valid: bool,
}

impl PriceStatus {
    /// Returns a zeroed, `valid: false` status used when a price could not be resolved.
    pub fn unusable() -> Self {
        Self {
            final_wad: 0,
            primary_wad: 0,
            secondary_wad: 0,
            price_timestamp: 0,
            stale: false,
            deviation: false,
            valid: false,
        }
    }
}

/// Typed, in-memory form of a resolved price: a `Wad` price alongside the asset's token
/// decimals and observation timestamp.
#[derive(Clone, Copy, Debug)]
pub struct PriceFeed {
    pub price: crate::math::fp::Wad,
    pub asset_decimals: u32,
    pub timestamp: u64,
}

impl PriceFeed {
    /// Converts a raw token amount (scaled by `asset_decimals`) to its WAD-scaled USD value
    /// at this feed's price.
    pub fn usd_value_wad(self, env: &Env, token_amount: i128) -> crate::math::fp::Wad {
        crate::math::fp::Wad::from_token(token_amount, self.asset_decimals).mul(env, self.price)
    }
}

impl From<&PriceFeedRaw> for PriceFeed {
    fn from(r: &PriceFeedRaw) -> Self {
        Self {
            price: crate::math::fp::Wad::from(r.price_wad),
            asset_decimals: r.asset_decimals,
            timestamp: r.timestamp,
        }
    }
}

impl From<&PriceFeed> for PriceFeedRaw {
    fn from(t: &PriceFeed) -> Self {
        Self {
            price_wad: t.price.raw(),
            asset_decimals: t.asset_decimals,
            timestamp: t.timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::WAD;

    #[test]
    fn test_price_feed_raw_typed_roundtrip() {
        let raw = PriceFeedRaw {
            price_wad: 12_345 * WAD,
            asset_decimals: 7,
            timestamp: 1_700_000_000,
        };
        let typed = PriceFeed::from(&raw);
        let back = PriceFeedRaw::from(&typed);
        assert_eq!(back.price_wad, raw.price_wad);
        assert_eq!(back.asset_decimals, raw.asset_decimals);
        assert_eq!(back.timestamp, raw.timestamp);
    }

    #[test]
    fn test_price_feed_usd_value_wad_scales_by_decimals() {
        let env = Env::default();
        let feed = PriceFeed {
            price: crate::math::fp::Wad::from(2 * WAD),
            asset_decimals: 7,
            timestamp: 0,
        };

        let usd = feed.usd_value_wad(&env, 100_000_000);
        assert_eq!(usd.raw(), 20 * WAD);
    }
}
