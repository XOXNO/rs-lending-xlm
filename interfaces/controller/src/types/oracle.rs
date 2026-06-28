use soroban_sdk::{contracttype, Address, Env, String};

pub use common::types::OracleAssetRef;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OraclePriceFluctuation {
    /// Upper bound for the primary/anchor ratio, in BPS.
    pub upper_ratio_bps: u32,
    /// Lower bound for the primary/anchor ratio, in BPS.
    pub lower_ratio_bps: u32,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum OracleProviderKind {
    ReflectorSep40 = 0,
    RedStonePriceFeed = 1,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

impl OracleSourceConfigInput {
    /// True when two configs read the same provider feed.
    pub fn reads_same_feed_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Reflector(x), Self::Reflector(y)) => {
                x.contract == y.contract && x.asset == y.asset && x.read_mode == y.read_mode
            }
            (Self::RedStone(x), Self::RedStone(y)) => {
                x.contract == y.contract && x.feed_id == y.feed_id
            }
            _ => false,
        }
    }
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

/// Quote base captured from a Reflector oracle at config time.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReflectorBase {
    Usd,
    Quoted(Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReflectorSourceConfig {
    pub contract: Address,
    pub asset: OracleAssetRef,
    pub read_mode: OracleReadMode,
    pub decimals: u32,
    /// Feed cadence, validated at listing time only (a config-time sanity bound).
    /// NOT consulted on the price-read path — runtime freshness is gated
    /// separately by `max_stale_seconds`/`is_stale`, so leaving this unread does
    /// not fail open. Do not assume it gates reads.
    pub resolution_seconds: u32,
    /// Quote base captured at config time; the read path reads this instead of
    /// calling the oracle's `base()`.
    pub base: ReflectorBase,
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
#[allow(clippy::large_enum_variant)]
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
            OracleSourceConfig::Reflector(config) => config.read_mode,
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
                upper_ratio_bps: 0,
                lower_ratio_bps: 0,
            },
            strategy: OracleStrategy::Single,
            primary: OracleSourceConfig::Reflector(ReflectorSourceConfig {
                contract: asset.clone(),
                asset: OracleAssetRef::Stellar(asset),
                read_mode: OracleReadMode::Spot,
                decimals,
                resolution_seconds: 0,
                // Pending sentinel is never read (PendingOracle rejects reads).
                base: ReflectorBase::Usd,
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
    pub tolerance_bps: u32,
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
    pub price: common::math::fp::Wad,
    pub asset_decimals: u32,
    pub timestamp: u64,
}

impl PriceFeed {
    pub fn usd_value_wad(self, env: &Env, token_amount: i128) -> common::math::fp::Wad {
        common::math::fp::Wad::from_token(token_amount, self.asset_decimals).mul(env, self.price)
    }
}

impl From<&PriceFeedRaw> for PriceFeed {
    fn from(r: &PriceFeedRaw) -> Self {
        Self {
            price: common::math::fp::Wad::from(r.price_wad),
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
    use common::constants::WAD;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Symbol;

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
            base: ReflectorBase::Usd,
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
    fn test_reads_same_feed_as_detects_duplicate_reflector_feed() {
        let env = Env::default();
        let contract = Address::generate(&env);
        let asset = OracleAssetRef::Stellar(Address::generate(&env));
        let a = OracleSourceConfigInput::Reflector(ReflectorSourceConfigInput {
            contract: contract.clone(),
            asset: asset.clone(),
            read_mode: OracleReadMode::Spot,
        });
        let b = OracleSourceConfigInput::Reflector(ReflectorSourceConfigInput {
            contract,
            asset,
            read_mode: OracleReadMode::Spot,
        });
        assert!(a.reads_same_feed_as(&b));
    }

    #[test]
    fn test_reads_same_feed_as_cross_provider_is_false() {
        let env = Env::default();
        let reflector = OracleSourceConfigInput::Reflector(reflector_input(&env));
        let redstone = OracleSourceConfigInput::RedStone(RedStoneSourceConfigInput {
            contract: Address::generate(&env),
            feed_id: String::from_str(&env, "ETH/USD"),
            max_stale_seconds: 900,
        });
        assert!(!reflector.reads_same_feed_as(&redstone));
        assert!(!redstone.reads_same_feed_as(&reflector));
    }

    #[test]
    fn test_oracle_type_shapes_roundtrip() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let _fluctuation = OraclePriceFluctuation {
            upper_ratio_bps: 200,
            lower_ratio_bps: 200,
        };
        let _provider_kinds = [
            OracleProviderKind::ReflectorSep40,
            OracleProviderKind::RedStonePriceFeed,
        ];
        let _assets = [
            OracleAssetRef::Stellar(asset.clone()),
            OracleAssetRef::Symbol(Symbol::new(&env, "USD")),
            OracleAssetRef::String(String::from_str(&env, "FEED")),
        ];
        let _modes = [OracleReadMode::Spot, OracleReadMode::Twap(3)];
        let _strategies = [OracleStrategy::Single, OracleStrategy::PrimaryWithAnchor];
        let quoted = ReflectorBase::Quoted(asset.clone());
        let mut resolved = reflector_resolved(&env);
        resolved.base = quoted;
        let _ = OracleSourceConfig::Reflector(resolved);
        let _ = OracleSourceConfig::RedStone(redstone_resolved(&env));
        let _input = MarketOracleConfigInput {
            max_price_stale_seconds: 900,
            tolerance_bps: 200,
            strategy: OracleStrategy::PrimaryWithAnchor,
            primary: OracleSourceConfigInput::Reflector(reflector_input(&env)),
            anchor: OracleSourceConfigInputOption::None,
            min_sanity_price_wad: WAD,
            max_sanity_price_wad: 100 * WAD,
        };
        let _market = MarketOracleConfig {
            asset_decimals: 7,
            max_price_stale_seconds: 900,
            tolerance: OraclePriceFluctuation {
                upper_ratio_bps: 200,
                lower_ratio_bps: 200,
            },
            strategy: OracleStrategy::Single,
            primary: OracleSourceConfig::Reflector(reflector_resolved(&env)),
            anchor: OracleSourceConfigOption::None,
            min_sanity_price_wad: 0,
            max_sanity_price_wad: 0,
        };
    }

    #[test]
    fn test_reads_same_feed_as_redstone_ignores_max_stale() {
        let env = Env::default();
        let contract = Address::generate(&env);
        let feed_id = String::from_str(&env, "BTC/USD");
        let a = OracleSourceConfigInput::RedStone(RedStoneSourceConfigInput {
            contract: contract.clone(),
            feed_id: feed_id.clone(),
            max_stale_seconds: 600,
        });
        let b = OracleSourceConfigInput::RedStone(RedStoneSourceConfigInput {
            contract,
            feed_id,
            max_stale_seconds: 900,
        });
        assert!(a.reads_same_feed_as(&b));
    }

    #[test]
    fn test_reads_same_feed_as_reflector_different_read_mode_is_false() {
        let env = Env::default();
        let contract = Address::generate(&env);
        let asset = OracleAssetRef::Stellar(Address::generate(&env));
        let spot = OracleSourceConfigInput::Reflector(ReflectorSourceConfigInput {
            contract: contract.clone(),
            asset: asset.clone(),
            read_mode: OracleReadMode::Spot,
        });
        let twap = OracleSourceConfigInput::Reflector(ReflectorSourceConfigInput {
            contract,
            asset,
            read_mode: OracleReadMode::Twap(5),
        });
        assert!(!spot.reads_same_feed_as(&twap));
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
            price: common::math::fp::Wad::from(2 * WAD), // $2/token
            asset_decimals: 7,
            timestamp: 0,
        };
        // 10 token at 7 decimals = 1e8 raw units; @ $2 = $20 in WAD.
        let usd = feed.usd_value_wad(&env, 100_000_000);
        assert_eq!(usd.raw(), 20 * WAD);
    }
}
