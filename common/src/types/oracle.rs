use soroban_sdk::{contracttype, Address, Env, String, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OraclePriceFluctuation {
    /// Upper bound against the primary source's first comparison, in BPS.
    pub first_upper_ratio_bps: u32,
    /// Lower bound against the primary source's first comparison, in BPS.
    pub first_lower_ratio_bps: u32,
    /// Upper bound for primary/anchor last comparison, in BPS.
    pub last_upper_ratio_bps: u32,
    /// Lower bound for primary/anchor last comparison, in BPS.
    pub last_lower_ratio_bps: u32,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum OracleProviderKind {
    ReflectorSep40 = 0,
    RedStonePriceFeed = 1,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleAssetRef {
    /// SEP-40 lookup by Stellar asset address.
    Stellar(Address),
    /// SEP-40 lookup by symbol.
    Symbol(Symbol),
    /// Provider-specific string identifier such as a RedStone feed id.
    String(String),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleReadMode {
    /// Read the latest provider price.
    Spot,
    /// Read a time-weighted average over the requested record count.
    Twap(u32),
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum OracleStrategy {
    /// Use only the primary source.
    Single = 0,
    /// Use primary plus anchor tolerance checks.
    PrimaryWithAnchor = 1,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReflectorSourceConfigInput {
    pub contract: Address,
    pub asset: OracleAssetRef,
    pub read_mode: OracleReadMode,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedStoneSourceConfigInput {
    pub contract: Address,
    pub feed_id: String,
    pub max_stale_seconds: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleSourceConfigInput {
    Reflector(ReflectorSourceConfigInput),
    RedStone(RedStoneSourceConfigInput),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleSourceConfigInputOption {
    None,
    Some(OracleSourceConfigInput),
}

impl OracleSourceConfigInputOption {
    pub fn as_ref(&self) -> Option<&OracleSourceConfigInput> {
        match self {
            Self::None => None,
            Self::Some(source) => Some(source),
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReflectorSourceConfig {
    pub contract: Address,
    pub asset: OracleAssetRef,
    pub read_mode: OracleReadMode,
    pub decimals: u32,
    pub resolution_seconds: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedStoneSourceConfig {
    pub contract: Address,
    pub feed_id: String,
    pub decimals: u32,
    pub max_stale_seconds: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleSourceConfig {
    Reflector(ReflectorSourceConfig),
    RedStone(RedStoneSourceConfig),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleSourceConfigOption {
    None,
    Some(OracleSourceConfig),
}

impl OracleSourceConfigOption {
    pub fn as_ref(&self) -> Option<&OracleSourceConfig> {
        match self {
            Self::None => None,
            Self::Some(source) => Some(source),
        }
    }
}

impl OracleSourceConfig {
    pub fn provider_kind(&self) -> OracleProviderKind {
        match self {
            OracleSourceConfig::Reflector(_) => OracleProviderKind::ReflectorSep40,
            OracleSourceConfig::RedStone(_) => OracleProviderKind::RedStonePriceFeed,
        }
    }

    pub fn read_mode(&self) -> OracleReadMode {
        match self {
            OracleSourceConfig::Reflector(config) => config.read_mode.clone(),
            OracleSourceConfig::RedStone(_) => OracleReadMode::Spot,
        }
    }

    pub fn decimals(&self) -> u32 {
        match self {
            OracleSourceConfig::Reflector(config) => config.decimals,
            OracleSourceConfig::RedStone(config) => config.decimals,
        }
    }

    pub fn max_stale_seconds(&self, default_max_stale_seconds: u64) -> u64 {
        match self {
            OracleSourceConfig::Reflector(_) => default_max_stale_seconds,
            OracleSourceConfig::RedStone(config) => config.max_stale_seconds,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketOracleConfig {
    /// Asset decimals used to convert token amounts before USD pricing.
    pub asset_decimals: u32,
    /// Default staleness limit for sources that do not carry their own limit.
    pub max_price_stale_seconds: u64,
    pub tolerance: OraclePriceFluctuation,
    pub strategy: OracleStrategy,
    pub primary: OracleSourceConfig,
    pub anchor: OracleSourceConfigOption,
    /// Inclusive lower sanity bound for final USD WAD price.
    pub min_sanity_price_wad: i128,
    /// Inclusive upper sanity bound for final USD WAD price.
    pub max_sanity_price_wad: i128,
}

impl MarketOracleConfig {
    pub fn pending_for(asset: Address, decimals: u32) -> Self {
        Self {
            asset_decimals: decimals,
            max_price_stale_seconds: 0,
            tolerance: OraclePriceFluctuation {
                first_upper_ratio_bps: 0,
                first_lower_ratio_bps: 0,
                last_upper_ratio_bps: 0,
                last_lower_ratio_bps: 0,
            },
            strategy: OracleStrategy::Single,
            primary: OracleSourceConfig::Reflector(ReflectorSourceConfig {
                contract: asset.clone(),
                asset: OracleAssetRef::Stellar(asset),
                read_mode: OracleReadMode::Spot,
                decimals,
                resolution_seconds: 0,
            }),
            anchor: OracleSourceConfigOption::None,
            min_sanity_price_wad: 0,
            max_sanity_price_wad: 0,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketOracleConfigInput {
    pub max_price_stale_seconds: u64,
    pub first_tolerance_bps: u32,
    pub last_tolerance_bps: u32,
    pub strategy: OracleStrategy,
    pub primary: OracleSourceConfigInput,
    pub anchor: OracleSourceConfigInputOption,
    pub min_sanity_price_wad: i128,
    pub max_sanity_price_wad: i128,
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

/// Typed oracle price used by controller math.
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
            price: crate::math::fp::Wad::from_raw(r.price_wad),
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
    use soroban_sdk::testutils::Address as _;

    fn reflector_input(env: &Env) -> ReflectorSourceConfigInput {
        ReflectorSourceConfigInput {
            contract: Address::generate(env),
            asset: OracleAssetRef::Stellar(Address::generate(env)),
            read_mode: OracleReadMode::Twap(5),
        }
    }

    fn reflector_resolved(env: &Env) -> ReflectorSourceConfig {
        ReflectorSourceConfig {
            contract: Address::generate(env),
            asset: OracleAssetRef::Stellar(Address::generate(env)),
            read_mode: OracleReadMode::Twap(5),
            decimals: 14,
            resolution_seconds: 300,
        }
    }

    fn redstone_resolved(env: &Env) -> RedStoneSourceConfig {
        RedStoneSourceConfig {
            contract: Address::generate(env),
            feed_id: String::from_str(env, "BTC/USD"),
            decimals: 8,
            max_stale_seconds: 900,
        }
    }

    #[test]
    fn test_input_option_none_is_none_and_as_ref_none() {
        let none = OracleSourceConfigInputOption::None;
        assert!(none.is_none());
        assert!(none.as_ref().is_none());
    }

    #[test]
    fn test_input_option_some_is_some_and_as_ref_yields_inner() {
        let env = Env::default();
        let some = OracleSourceConfigInputOption::Some(OracleSourceConfigInput::Reflector(
            reflector_input(&env),
        ));
        assert!(!some.is_none());
        assert!(matches!(
            some.as_ref(),
            Some(OracleSourceConfigInput::Reflector(_))
        ));
    }

    #[test]
    fn test_resolved_option_as_ref_branches() {
        let env = Env::default();
        let none = OracleSourceConfigOption::None;
        assert!(none.as_ref().is_none());

        let some =
            OracleSourceConfigOption::Some(OracleSourceConfig::Reflector(reflector_resolved(&env)));
        assert!(matches!(
            some.as_ref(),
            Some(OracleSourceConfig::Reflector(_))
        ));
    }

    #[test]
    fn test_oracle_source_config_reflector_accessors() {
        let env = Env::default();
        let cfg = OracleSourceConfig::Reflector(reflector_resolved(&env));
        assert_eq!(cfg.provider_kind(), OracleProviderKind::ReflectorSep40);
        assert_eq!(cfg.read_mode(), OracleReadMode::Twap(5));
        assert_eq!(cfg.decimals(), 14);
        // Reflector falls back to the market-level default for staleness.
        assert_eq!(cfg.max_stale_seconds(900), 900);
    }

    #[test]
    fn test_oracle_source_config_redstone_accessors() {
        let env = Env::default();
        let cfg = OracleSourceConfig::RedStone(redstone_resolved(&env));
        assert_eq!(cfg.provider_kind(), OracleProviderKind::RedStonePriceFeed);
        // Redstone collapses to spot — it doesn't carry a read-mode field.
        assert_eq!(cfg.read_mode(), OracleReadMode::Spot);
        assert_eq!(cfg.decimals(), 8);
        // Redstone uses its own per-source max-stale, ignoring the default.
        assert_eq!(cfg.max_stale_seconds(60), 900);
    }

    #[test]
    fn test_market_oracle_config_pending_for_shape() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let cfg = MarketOracleConfig::pending_for(asset.clone(), 7);

        assert_eq!(cfg.asset_decimals, 7);
        assert_eq!(cfg.max_price_stale_seconds, 0);
        assert_eq!(cfg.strategy, OracleStrategy::Single);
        assert_eq!(cfg.min_sanity_price_wad, 0);
        assert_eq!(cfg.max_sanity_price_wad, 0);
        assert!(cfg.anchor.as_ref().is_none());

        // The sentinel `contract` self-points at the asset; runtime callers
        // reject this via the market-status guard in `oracle::price`.
        match cfg.primary {
            OracleSourceConfig::Reflector(r) => {
                assert_eq!(r.contract, asset);
                assert_eq!(r.read_mode, OracleReadMode::Spot);
                assert_eq!(r.decimals, 7);
            }
            _ => panic!("pending_for must build a Reflector primary"),
        }
    }

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
            price: crate::math::fp::Wad::from_raw(2 * WAD), // $2/token
            asset_decimals: 7,
            timestamp: 0,
        };
        // 10 token at 7 decimals = 1e8 raw units; @ $2 = $20 in WAD.
        let usd = feed.usd_value_wad(&env, 100_000_000);
        assert_eq!(usd.raw(), 20 * WAD);
    }
}
