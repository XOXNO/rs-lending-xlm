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

    pub asset_decimals: u32,

    pub timestamp: u64,
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
            price: crate::math::fp::Wad::from(2 * WAD),
            asset_decimals: 7,
            timestamp: 0,
        };

        let usd = feed.usd_value_wad(&env, 100_000_000);
        assert_eq!(usd.raw(), 20 * WAD);
    }
}
