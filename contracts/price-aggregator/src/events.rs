//! Oracle config event schema. `UpdateAssetOracleEvent` publishes a full
//! `EventOracleProvider` snapshot on every config change, mirroring the wire
//! shape off-chain decoders already consume.

use soroban_sdk::{contractevent, contracttype, symbol_short, Address, Env, String, Symbol};

use common::types::{
    AssetOracle, AssetOracleConfig, OracleAssetRef, OracleProviderKind, OracleReadMode,
    OracleSourceConfig, OracleStrategy, OracleTolerance, PriceKey, ReflectorBase,
};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum EventOracleType {
    None = 0,
    Normal = 1,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum EventPricingMethod {
    None = 0,
    Safe = 1,
    Instant = 2,
    Aggregator = 3,
    Mix = 4,
}

impl From<OracleStrategy> for EventPricingMethod {
    fn from(value: OracleStrategy) -> Self {
        match value {
            OracleStrategy::Single => Self::Instant,
            OracleStrategy::PrimaryWithAnchor => Self::Mix,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EventOracleProvider {
    pub base_token_id: Address,
    pub quote_token_id: Symbol,
    pub tolerance: OracleTolerance,
    pub pricing_method: EventPricingMethod,
    pub oracle_type: EventOracleType,
    pub strategy: u32,
    pub asset_decimals: u32,
    pub max_price_stale_seconds: u64,
    pub primary_provider: u32,
    pub primary_contract: Address,
    pub primary_asset: Option<Address>,
    pub primary_symbol: Option<Symbol>,
    pub primary_feed_id: Option<String>,
    pub primary_quote_token: Option<Address>,
    pub primary_read_mode: u32,
    pub primary_twap_records: u32,
    pub primary_decimals: u32,
    pub primary_resolution_seconds: u32,
    pub primary_max_stale_seconds: u64,
    pub anchor_provider: Option<u32>,
    pub anchor_contract: Option<Address>,
    pub anchor_asset: Option<Address>,
    pub anchor_symbol: Option<Symbol>,
    pub anchor_feed_id: Option<String>,
    pub anchor_quote_token: Option<Address>,
    pub anchor_read_mode: u32,
    pub anchor_twap_records: u32,
    pub anchor_decimals: u32,
    pub anchor_resolution_seconds: u32,
    pub anchor_max_stale_seconds: u64,
    pub min_sanity_price_wad: i128,
    pub max_sanity_price_wad: i128,
}

impl EventOracleProvider {
    /// Flattens an `AssetOracleConfig` into the wire snapshot decoders consume:
    /// the primary and anchor legs become parallel field sets and the quote is
    /// always `USD`.
    ///
    /// With no anchor the optional anchor fields are `None` and the scalar ones
    /// are `0`. Each leg reports the stale window it is actually read under, so
    /// a Reflector leg carries the market-wide `max_price_stale_seconds` and a
    /// RedStone/Xoxno leg carries its own.
    pub fn from_oracle(asset: &Address, oracle: &AssetOracleConfig) -> Self {
        let market_max_stale = oracle.max_price_stale_seconds;
        let primary = EventOracleSource::from(&oracle.primary, market_max_stale);
        let anchor = oracle
            .anchor
            .as_ref()
            .map(|source| EventOracleSource::from(source, market_max_stale));

        Self {
            base_token_id: asset.clone(),
            quote_token_id: symbol_short!("USD"),
            tolerance: oracle.tolerance.clone(),
            pricing_method: oracle.strategy.into(),
            oracle_type: EventOracleType::Normal,
            strategy: oracle.strategy as u32,
            asset_decimals: oracle.asset_decimals,
            max_price_stale_seconds: oracle.max_price_stale_seconds,
            primary_provider: primary.provider,
            primary_contract: primary.contract,
            primary_asset: primary.asset,
            primary_symbol: primary.symbol,
            primary_feed_id: primary.feed_id,
            primary_quote_token: primary.quote_token,
            primary_read_mode: primary.read_mode,
            primary_twap_records: primary.twap_records,
            primary_decimals: primary.decimals,
            primary_resolution_seconds: primary.resolution_seconds,
            primary_max_stale_seconds: primary.max_stale_seconds,
            anchor_provider: anchor.as_ref().map(|source| source.provider),
            anchor_contract: anchor.as_ref().map(|source| source.contract.clone()),
            anchor_asset: anchor.as_ref().and_then(|source| source.asset.clone()),
            anchor_symbol: anchor.as_ref().and_then(|source| source.symbol.clone()),
            anchor_feed_id: anchor.as_ref().and_then(|source| source.feed_id.clone()),
            anchor_quote_token: anchor
                .as_ref()
                .and_then(|source| source.quote_token.clone()),
            anchor_read_mode: anchor_or_zero(anchor.as_ref(), |source| source.read_mode),
            anchor_twap_records: anchor_or_zero(anchor.as_ref(), |source| source.twap_records),
            anchor_decimals: anchor_or_zero(anchor.as_ref(), |source| source.decimals),
            anchor_resolution_seconds: anchor_or_zero(anchor.as_ref(), |source| {
                source.resolution_seconds
            }),
            anchor_max_stale_seconds: anchor_or_zero(anchor.as_ref(), |source| {
                source.max_stale_seconds
            }),
            min_sanity_price_wad: oracle.min_sanity_price_wad,
            max_sanity_price_wad: oracle.max_sanity_price_wad,
        }
    }
}

fn anchor_or_zero<T: Default>(
    anchor: Option<&EventOracleSource>,
    pick: impl Fn(&EventOracleSource) -> T,
) -> T {
    anchor.map(pick).unwrap_or_default()
}

struct EventOracleSource {
    provider: u32,
    contract: Address,
    asset: Option<Address>,
    symbol: Option<Symbol>,
    feed_id: Option<String>,
    quote_token: Option<Address>,
    read_mode: u32,
    twap_records: u32,
    decimals: u32,
    resolution_seconds: u32,
    max_stale_seconds: u64,
}

impl EventOracleSource {
    fn from(source: &OracleSourceConfig, market_max_stale_seconds: u64) -> Self {
        match source {
            OracleSourceConfig::Reflector(config) => {
                let (asset, symbol, feed_id) = match &config.asset {
                    OracleAssetRef::Stellar(asset) => (Some(asset.clone()), None, None),
                    OracleAssetRef::Symbol(symbol) => (None, Some(symbol.clone()), None),
                    OracleAssetRef::String(feed_id) => (None, None, Some(feed_id.clone())),
                };
                let (read_mode, twap_records) = read_mode_parts(&config.read_mode);
                let quote_token = match &config.base {
                    ReflectorBase::Usd => None,
                    ReflectorBase::Quoted(quote) => Some(quote.clone()),
                };
                Self {
                    provider: OracleProviderKind::ReflectorSep40 as u32,
                    contract: config.contract.clone(),
                    asset,
                    symbol,
                    feed_id,
                    quote_token,
                    read_mode,
                    twap_records,
                    decimals: config.decimals,
                    resolution_seconds: config.resolution_seconds,
                    max_stale_seconds: market_max_stale_seconds,
                }
            }
            OracleSourceConfig::RedStone(config) | OracleSourceConfig::Xoxno(config) => Self {
                provider: source.provider_kind() as u32,
                contract: config.contract.clone(),
                asset: None,
                symbol: None,
                feed_id: Some(config.feed_id.clone()),
                quote_token: None,
                read_mode: 0,
                twap_records: 0,
                decimals: config.decimals,
                resolution_seconds: 0,
                max_stale_seconds: config.max_stale_seconds,
            },
        }
    }
}

fn read_mode_parts(read_mode: &OracleReadMode) -> (u32, u32) {
    match read_mode {
        OracleReadMode::Spot => (0, 0),
        OracleReadMode::Twap(records) => (1, *records),
    }
}

/// Published on every composable-oracle write.
///
/// Carries the config verbatim rather than a projection. Two of the model's
/// guarantees are *disclosure* mechanisms rather than on-chain checks -
/// `IndependencePolicy::AllowShared` names the trust a config knowingly shares,
/// and `FeedNature::Fundamental` exempts a feed from the smoothing rule - and a
/// disclosure nobody can observe is not a disclosure. This is the half that
/// makes them mean something: an indexer can alarm on a waiver appearing, or on
/// a market feed being declared fundamental.
#[contractevent(topics = ["config", "asset_oracle"])]
#[derive(Clone, Debug)]
pub struct UpdateAssetOracleV2Event {
    pub key: PriceKey,
    pub oracle: AssetOracle,
}

/// Publishes the stored composable oracle. `config::set_asset_oracle` calls this
/// once, after the write.
///
/// # Events
/// * topics — `["config", "asset_oracle"]`
pub(crate) fn emit_asset_oracle_updated(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    UpdateAssetOracleV2Event {
        key: key.clone(),
        oracle: oracle.clone(),
    }
    .publish(env);
}

#[cfg(test)]
#[path = "../tests/oracle/events.rs"]
mod tests;
