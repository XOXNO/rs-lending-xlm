use soroban_sdk::{contracttype, Address, String, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OraclePriceFluctuation {
    pub first_upper_ratio_bps: u32,
    pub first_lower_ratio_bps: u32,
    pub last_upper_ratio_bps: u32,
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
    Stellar(Address),
    Symbol(Symbol),
    String(String),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleReadMode {
    Spot,
    Twap(u32),
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum OracleStrategy {
    Single = 0,
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
    pub asset_decimals: u32,
    pub max_price_stale_seconds: u64,
    pub tolerance: OraclePriceFluctuation,
    pub strategy: OracleStrategy,
    pub primary: OracleSourceConfig,
    pub anchor: OracleSourceConfigOption,
    pub min_sanity_price_wad: i128,
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

#[contracttype]
#[derive(Clone, Debug)]
pub struct PriceFeed {
    pub price_wad: i128,
    pub asset_decimals: u32,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SafePriceFeed {
    pub price_wad: i128,
    pub asset_decimals: u32,
    pub within_first_tolerance: bool,
    pub within_second_tolerance: bool,
}
