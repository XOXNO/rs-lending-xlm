use soroban_sdk::{contracttype, Address, Env, String, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleAssetRef {
    Stellar(Address),

    Symbol(Symbol),

    String(String),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleTolerance {
    pub upper_ratio_bps: u32,

    pub lower_ratio_bps: u32,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OracleReadMode {
    Spot,

    Twap(u32),
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PriceFeedRaw {
    pub price_wad: i128,

    /// Lowest and highest leg behind `price_wad`; equal to it for a single
    /// source. A dual source reports the interval its legs actually spanned, so
    /// a consumer can price each side of a position against the edge that is
    /// conservative for it instead of the midpoint, which hands whoever controls
    /// one leg half the tolerance band.
    pub low_wad: i128,

    pub high_wad: i128,

    pub asset_decimals: u32,

    pub timestamp: u64,
}

impl PriceFeedRaw {
    /// Value collateral at the low edge and debt at the high edge, so a leg that
    /// drifts within the tolerance band can only understate account health.
    #[must_use]
    pub fn collateral_wad(&self) -> i128 {
        self.low_wad
    }

    #[must_use]
    pub fn debt_wad(&self) -> i128 {
        self.high_wad
    }
}

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

#[derive(Clone, Copy, Debug)]
pub struct PriceFeed {
    pub price: crate::math::fp::Wad,
    pub low: crate::math::fp::Wad,
    pub high: crate::math::fp::Wad,
    pub asset_decimals: u32,
    pub timestamp: u64,
}

impl PriceFeed {
    pub fn usd_value_wad(self, env: &Env, token_amount: i128) -> crate::math::fp::Wad {
        crate::math::fp::Wad::from_token(token_amount, self.asset_decimals).mul(env, self.price)
    }

    /// Collateral is valued at the low leg and debt at the high leg. For a single
    /// source both equal `price`; for a dual source this spends the tolerance
    /// band against the account instead of handing it to whoever moved a leg.
    pub fn collateral_value_wad(self, env: &Env, token_amount: i128) -> crate::math::fp::Wad {
        crate::math::fp::Wad::from_token(token_amount, self.asset_decimals).mul(env, self.low)
    }

    pub fn debt_value_wad(self, env: &Env, token_amount: i128) -> crate::math::fp::Wad {
        crate::math::fp::Wad::from_token(token_amount, self.asset_decimals).mul(env, self.high)
    }
}

impl From<&PriceFeedRaw> for PriceFeed {
    fn from(r: &PriceFeedRaw) -> Self {
        Self {
            price: crate::math::fp::Wad::from(r.price_wad),
            low: crate::math::fp::Wad::from(r.low_wad),
            high: crate::math::fp::Wad::from(r.high_wad),
            asset_decimals: r.asset_decimals,
            timestamp: r.timestamp,
        }
    }
}

impl From<&PriceFeed> for PriceFeedRaw {
    fn from(t: &PriceFeed) -> Self {
        Self {
            price_wad: t.price.raw(),
            low_wad: t.low.raw(),
            high_wad: t.high.raw(),
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
            low_wad: 12_345 * WAD,
            high_wad: 12_345 * WAD,
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
            low: crate::math::fp::Wad::from(19 * WAD / 10),
            high: crate::math::fp::Wad::from(21 * WAD / 10),
            asset_decimals: 7,
            timestamp: 0,
        };

        let usd = feed.usd_value_wad(&env, 100_000_000);
        assert_eq!(usd.raw(), 20 * WAD);
    }

    // Collateral takes the low leg and debt the high leg, so a leg drifting
    // inside the tolerance band can only understate health, never overstate it.
    #[test]
    fn test_price_feed_values_collateral_low_and_debt_high() {
        let env = Env::default();
        let feed = PriceFeed {
            price: crate::math::fp::Wad::from(2 * WAD),
            low: crate::math::fp::Wad::from(19 * WAD / 10),
            high: crate::math::fp::Wad::from(21 * WAD / 10),
            asset_decimals: 7,
            timestamp: 0,
        };

        assert_eq!(feed.collateral_value_wad(&env, 100_000_000).raw(), 19 * WAD);
        assert_eq!(feed.debt_value_wad(&env, 100_000_000).raw(), 21 * WAD);
    }
}
