//! Oracle config and price-feed types (SEP-40 refs, per-source/market config).

use soroban_sdk::{contracttype, Address, Env, String, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleAssetRef {
    /// SEP-40 lookup by Stellar asset address.
    Stellar(Address),
    /// SEP-40 lookup by symbol.
    Symbol(Symbol),
    /// Unused by Reflector/RedStone (rejected / not mapped).
    String(String),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleTolerance {
    /// Upper bound for the primary/anchor ratio, in BPS.
    pub upper_ratio_bps: u32,
    /// Lower bound for the primary/anchor ratio, in BPS.
    pub lower_ratio_bps: u32,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OracleReadMode {
    /// Read the latest provider price.
    Spot,
    /// Read a time-weighted average over the requested record count.
    Twap(u32),
}

/// Oracle price payload embedded in liquidation entries and events.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PriceFeedRaw {
    /// USD price in WAD.
    pub price_wad: i128,
    /// Token decimals used for amount-to-WAD conversion.
    pub asset_decimals: u32,
    /// Provider timestamp accepted by oracle policy.
    pub timestamp: u64,
}

/// Soft diagnostic snapshot of a token-rooted USD price for views.
///
/// Unlike `prices` / `price`, this does not revert on stale or out-of-band legs.
/// `valid` is true only when the price would pass the fail-closed solvency path
/// (fresh, in tolerance when anchored, positive, within sanity).
///
/// ABI note: `secondary_wad` is the anchor leg (equals final/primary under
/// PrimaryOnly); it is not a swap-aggregator price.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceStatus {
    /// Final composed USD WAD (midpoint when dual legs exist and can be blended).
    pub final_wad: i128,
    /// Primary oracle leg, USD WAD.
    pub primary_wad: i128,
    /// Anchor when dual-source; equals final/primary under PrimaryOnly (USD WAD).
    pub secondary_wad: i128,
    /// Freshness timestamp of the final blend (min of legs), seconds.
    pub price_timestamp: u64,
    /// True when any required leg is past its max-stale window.
    pub stale: bool,
    /// True when dual legs disagree outside the configured tolerance band.
    pub deviation: bool,
    /// Usable for solvency-style decisions: `!stale && !deviation` and gates pass.
    pub valid: bool,
}

impl PriceStatus {
    /// Zeroed unusable status (missing config, unreadable feed, …).
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

/// Typed oracle price used by controller math (USD WAD).
#[derive(Clone, Copy, Debug)]
pub struct PriceFeed {
    /// USD price, WAD.
    pub price: crate::math::fp::Wad,
    pub asset_decimals: u32,
    pub timestamp: u64,
}

impl PriceFeed {
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
            price: crate::math::fp::Wad::from(2 * WAD), // $2/token
            asset_decimals: 7,
            timestamp: 0,
        };
        // 10 token at 7 decimals = 1e8 raw units; @ $2 = $20 in WAD.
        let usd = feed.usd_value_wad(&env, 100_000_000);
        assert_eq!(usd.raw(), 20 * WAD);
    }
}
