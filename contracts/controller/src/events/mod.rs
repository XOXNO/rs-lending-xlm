use soroban_sdk::{contractevent, contracttype, symbol_short, Address, Env, String, Symbol, Vec};

use common::types::{
    Account, AccountMeta, AccountPosition, DebtPosition, MarketOracleConfig, OracleAssetRef,
    OraclePriceFluctuation, OracleProviderKind, OracleReadMode, OracleSourceConfig, OracleStrategy,
    PositionMode, ReflectorBase,
};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum EventPositionMode {
    None = 0,
    Multiply = 1,
    Long = 2,
    Short = 3,
}

impl From<PositionMode> for EventPositionMode {
    fn from(value: PositionMode) -> Self {
        match value {
            PositionMode::Normal => Self::None,
            PositionMode::Multiply => Self::Multiply,
            PositionMode::Long => Self::Long,
            PositionMode::Short => Self::Short,
        }
    }
}

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
#[derive(Clone, Debug, Eq, PartialEq)]
/// Account attributes, vec-encoded inside the batch position event.
///
/// Field order is wire ABI; do not reorder:
/// `[owner, spoke_id, mode]`.
pub struct EventAccountAttributes(pub Address, pub u32, pub EventPositionMode);

impl From<&Account> for EventAccountAttributes {
    fn from(value: &Account) -> Self {
        Self(value.owner.clone(), value.spoke_id, value.mode.into())
    }
}

impl From<&AccountMeta> for EventAccountAttributes {
    fn from(value: &AccountMeta) -> Self {
        Self(value.owner.clone(), value.spoke_id, value.mode.into())
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EventOracleProvider {
    pub base_token_id: Address,
    pub quote_token_id: Symbol,
    pub tolerance: OraclePriceFluctuation,
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
    pub fn from_oracle(_env: &Env, asset: &Address, oracle: &MarketOracleConfig) -> Self {
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

/// Reads a numeric anchor field, defaulting to zero when no anchor source exists.
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
    /// Quote currency the feed is denominated in: `None` for USD-quoted feeds,
    /// `Some(token)` for feeds quoted via a Stellar token (e.g. USDC SAC) that
    /// the contract reprices to USD. `None` for RedStone (USD by feed id).
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
            OracleSourceConfig::RedStone(config) => Self {
                provider: OracleProviderKind::RedStonePriceFeed as u32,
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

#[contractevent(topics = ["market", "create"])]
#[derive(Clone, Debug)]
pub struct CreateMarketEvent {
    pub hub_id: u32,
    pub base_asset: Address,
    pub max_borrow_rate: i128,
    pub base_borrow_rate: i128,
    pub slope1: i128,
    pub slope2: i128,
    pub slope3: i128,
    pub mid_utilization: i128,
    pub optimal_utilization: i128,
    pub max_utilization: i128,
    pub reserve_factor: u32,
    pub market_address: Address,
}

#[contractevent(topics = ["market", "params_update"])]
#[derive(Clone, Debug)]
pub struct UpdateMarketParamsEvent {
    pub asset: Address,
    pub max_borrow_rate: i128,
    pub base_borrow_rate: i128,
    pub slope1: i128,
    pub slope2: i128,
    pub slope3: i128,
    pub mid_utilization: i128,
    pub optimal_utilization: i128,
    pub max_utilization: i128,
    pub reserve_factor: u32,
}

/// Position action stored as a stable `u32` discriminant.
/// Off-chain decoders depend on these values.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PositionAction {
    Supply = 0,
    Borrow = 1,
    Withdraw = 2,
    Repay = 3,
    LiqRepay = 4,
    LiqSeize = 5,
    Multiply = 6,
    ParamUpd = 7,
    SwDebtR = 8,
    SwColWd = 9,
    RpColWd = 10,
    RpColR = 11,
    CloseWd = 12,
    Migrate = 13,
}

/// Collateral-side position delta, vec-encoded for client compatibility.
/// Field order is wire ABI; do not reorder.
/// Risk params are the position entry values.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EventDepositDelta(
    pub PositionAction,
    pub u32,
    pub Address,
    pub i128,
    pub i128,
    pub i128,
    pub u32,
    pub u32,
    pub u32,
);

impl EventDepositDelta {
    pub fn new(
        action: PositionAction,
        hub_id: u32,
        asset: Address,
        index_ray: i128,
        amount: i128,
        position: &AccountPosition,
    ) -> Self {
        Self(
            action,
            hub_id,
            asset,
            position.scaled_amount.raw(),
            index_ray,
            amount,
            position.liquidation_threshold.raw() as u32,
            position.liquidation_bonus.raw() as u32,
            position.loan_to_value.raw() as u32,
        )
    }
}

/// Debt-side position delta; no collateral risk params on this side.
///
/// Field order is wire ABI; do not reorder:
/// `[action, asset, scaled_amount, index_ray, amount]`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EventBorrowDelta(
    pub PositionAction,
    pub u32,
    pub Address,
    pub i128,
    pub i128,
    pub i128,
);

impl EventBorrowDelta {
    pub fn new(
        action: PositionAction,
        hub_id: u32,
        asset: Address,
        index_ray: i128,
        amount: i128,
        position: &DebtPosition,
    ) -> Self {
        Self(
            action,
            hub_id,
            asset,
            position.scaled_amount.raw(),
            index_ray,
            amount,
        )
    }
}

#[contractevent(topics = ["position", "batch_update"], data_format = "vec")]
#[derive(Clone, Debug)]
pub struct UpdatePositionBatchEvent {
    /// Account whose positions changed.
    pub account_id: u64,
    pub account_attributes: EventAccountAttributes,
    /// Collateral-side deltas recorded during the successful transaction.
    pub deposits: Vec<EventDepositDelta>,
    /// Debt-side deltas recorded during the successful transaction.
    pub borrows: Vec<EventBorrowDelta>,
}

mod config;
mod debt;
mod flash;
mod strategy;

pub use config::*;
pub use debt::*;
pub use flash::*;
pub use strategy::*;

#[cfg(test)]
#[path = "../../tests/events.rs"]
mod tests;
