use soroban_sdk::{contractevent, contracttype, symbol_short, Address, Env, String, Symbol, Vec};

use controller_interface::types::{
    Account, AccountMeta, AccountPosition, AssetConfigRaw, DebtPosition, EModeAssetConfig,
    EModeCategoryRaw, MarketConfig, OracleAssetRef, OraclePriceFluctuation, OracleProviderKind,
    OracleReadMode, OracleSourceConfig, OracleStrategy, PositionMode, ReflectorBase,
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

/// Account attributes attached to position batch events.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
/// Account attributes, vec-encoded inside the batch position event.
///
/// Field order is wire ABI — never reorder:
/// `[owner, e_mode_category_id, is_isolated_position, mode, isolated_token]`.
pub struct EventAccountAttributes(
    pub Address,
    pub u32,
    pub bool,
    pub EventPositionMode,
    pub Option<Address>,
);

impl EventAccountAttributes {
    fn build(
        owner: &Address,
        is_isolated: bool,
        e_mode_category_id: u32,
        mode: PositionMode,
        isolated_asset: &Option<Address>,
    ) -> Self {
        Self(
            owner.clone(),
            e_mode_category_id,
            is_isolated,
            mode.into(),
            isolated_asset.clone(),
        )
    }
}

impl From<&Account> for EventAccountAttributes {
    fn from(value: &Account) -> Self {
        Self::build(
            &value.owner,
            value.is_isolated,
            value.e_mode_category_id,
            value.mode,
            &value.isolated_asset,
        )
    }
}

impl From<&AccountMeta> for EventAccountAttributes {
    fn from(value: &AccountMeta) -> Self {
        Self::build(
            &value.owner,
            value.is_isolated,
            value.e_mode_category_id,
            value.mode,
            &value.isolated_asset,
        )
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
    pub fn from_market(_env: &Env, asset: &Address, market: &MarketConfig) -> Self {
        let market_max_stale = market.oracle_config.max_price_stale_seconds;
        let primary = EventOracleSource::from(&market.oracle_config.primary, market_max_stale);
        let anchor = market
            .oracle_config
            .anchor
            .as_ref()
            .map(|source| EventOracleSource::from(source, market_max_stale));

        Self {
            base_token_id: asset.clone(),
            quote_token_id: symbol_short!("USD"),
            tolerance: market.oracle_config.tolerance.clone(),
            pricing_method: market.oracle_config.strategy.into(),
            oracle_type: EventOracleType::Normal,
            strategy: market.oracle_config.strategy as u32,
            asset_decimals: market.oracle_config.asset_decimals,
            max_price_stale_seconds: market.oracle_config.max_price_stale_seconds,
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
            min_sanity_price_wad: market.oracle_config.min_sanity_price_wad,
            max_sanity_price_wad: market.oracle_config.max_sanity_price_wad,
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
    pub config: AssetConfigRaw,
}

#[contractevent(topics = ["market", "params_update"])]
#[derive(Clone, Debug)]
pub struct UpdateMarketParamsEvent {
    pub asset: Address,
    pub max_borrow_rate_ray: i128,
    pub base_borrow_rate_ray: i128,
    pub slope1_ray: i128,
    pub slope2_ray: i128,
    pub slope3_ray: i128,
    pub mid_utilization_ray: i128,
    pub optimal_utilization_ray: i128,
    pub max_utilization_ray: i128,
    pub reserve_factor_bps: u32,
}

/// Per-market accrual snapshot, vec-encoded for the batch market event.
///
/// Field order is wire ABI — never reorder:
/// `[asset, timestamp, supply_index_ray, borrow_index_ray, reserves_ray,
///   supplied_ray, borrowed_ray, revenue_ray, asset_price_wad]`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EventMarketState(
    pub Address,
    pub u64,
    pub i128,
    pub i128,
    pub i128,
    pub i128,
    pub i128,
    pub i128,
    pub Option<i128>,
);

impl From<&controller_interface::types::MarketStateSnapshot> for EventMarketState {
    fn from(s: &controller_interface::types::MarketStateSnapshot) -> Self {
        Self(
            s.asset.clone(),
            s.timestamp,
            s.supply_index_ray,
            s.borrow_index_ray,
            s.reserves_ray,
            s.supplied_ray,
            s.borrowed_ray,
            s.revenue_ray,
            s.asset_price_wad,
        )
    }
}

#[contractevent(topics = ["market", "batch_state_update"], data_format = "single-value")]
#[derive(Clone, Debug)]
pub struct UpdateMarketStateBatchEvent {
    /// Pool accrual and accounting snapshots emitted after a successful batch.
    pub updates: Vec<EventMarketState>,
}

/// Action that produced a position delta. The `u32` discriminants are wire
/// ABI: the off-chain decoder maps them back to the legacy action strings,
/// so variants must never be renumbered or removed.
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
}

/// Collateral-side position delta, vec-encoded for the batch position event.
///
/// Field order is wire ABI — never reorder:
/// `[action, asset, scaled_amount_ray, index_ray, amount,
///   liquidation_threshold_bps, liquidation_bonus_bps, loan_to_value_bps]`.
/// The risk params are the position's entry values (e-mode adjusted).
#[contracttype]
#[derive(Clone, Debug)]
pub struct EventDepositDelta(
    pub PositionAction,
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
        asset: Address,
        index_ray: i128,
        amount: i128,
        position: &AccountPosition,
    ) -> Self {
        Self(
            action,
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

/// Debt-side position delta — no collateral risk params on this side.
///
/// Field order is wire ABI — never reorder:
/// `[action, asset, scaled_amount_ray, index_ray, amount]`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EventBorrowDelta(
    pub PositionAction,
    pub Address,
    pub i128,
    pub i128,
    pub i128,
);

impl EventBorrowDelta {
    pub fn new(
        action: PositionAction,
        asset: Address,
        index_ray: i128,
        amount: i128,
        position: &DebtPosition,
    ) -> Self {
        Self(
            action,
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

#[contractevent(topics = ["position", "flash_loan"])]
#[derive(Clone, Debug)]
pub struct FlashLoanEvent {
    pub asset: Address,
    pub receiver: Address,
    pub caller: Address,
    pub amount: i128,
    pub fee: i128,
}

#[contractevent(topics = ["config", "asset"])]
#[derive(Clone, Debug)]
pub struct UpdateAssetConfigEvent {
    pub asset: Address,
    pub config: AssetConfigRaw,
}

#[contractevent(topics = ["config", "oracle"])]
#[derive(Clone, Debug)]
pub struct UpdateAssetOracleEvent {
    pub asset: Address,
    pub oracle: EventOracleProvider,
}

/// E-mode category snapshot emitted after category changes.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EventEModeCategory {
    pub category_id: u32,
    pub loan_to_value_bps: u32,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub is_deprecated: bool,
}

impl EventEModeCategory {
    pub fn new(category_id: u32, category: &EModeCategoryRaw) -> Self {
        Self {
            category_id,
            loan_to_value_bps: category.loan_to_value_bps,
            liquidation_threshold_bps: category.liquidation_threshold_bps,
            liquidation_bonus_bps: category.liquidation_bonus_bps,
            is_deprecated: category.is_deprecated,
        }
    }
}

#[contractevent(topics = ["config", "emode_category"])]
#[derive(Clone, Debug)]
pub struct UpdateEModeCategoryEvent {
    pub category: EventEModeCategory,
}

#[contractevent(topics = ["config", "emode_asset"])]
#[derive(Clone, Debug)]
pub struct UpdateEModeAssetEvent {
    pub asset: Address,
    pub config: EModeAssetConfig,
    pub category_id: u32,
}

#[contractevent(topics = ["config", "remove_emode_asset"])]
#[derive(Clone, Debug)]
pub struct RemoveEModeAssetEvent {
    pub asset: Address,
    pub category_id: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
/// Field order is wire ABI: `[asset, total_debt_usd_wad]`.
pub struct EventDebtCeilingEntry(pub Address, pub i128);

#[contractevent(topics = ["debt", "ceiling_batch_update"], data_format = "single-value")]
#[derive(Clone, Debug)]
pub struct UpdateDebtCeilingBatchEvent {
    /// Final isolated-debt totals for assets touched in the transaction.
    pub updates: Vec<EventDebtCeilingEntry>,
}

#[contractevent(topics = ["debt", "bad_debt"])]
#[derive(Clone, Debug)]
pub struct CleanBadDebtEvent {
    pub account_id: u64,
    /// Debt written off by cleanup, in USD WAD.
    pub total_borrow_usd_wad: i128,
    /// Collateral seized by cleanup, in USD WAD.
    pub total_collateral_usd_wad: i128,
}

#[contractevent(topics = ["strategy", "initial_payment"])]
#[derive(Clone, Debug)]
pub struct InitialMultiplyPaymentEvent {
    pub token: Address,
    pub amount: i128,
    pub usd_value_wad: i128,
    pub account_id: u64,
}

#[contractevent(topics = ["config", "approve_token"])]
#[derive(Clone, Debug)]
pub struct ApproveTokenEvent {
    pub wasm_hash: soroban_sdk::BytesN<32>,
    pub approved: bool,
}

#[contractevent(topics = ["config", "aggregator"])]
#[derive(Clone, Debug)]
pub struct UpdateAggregatorEvent {
    pub aggregator: Address,
}

#[contractevent(topics = ["config", "accumulator"])]
#[derive(Clone, Debug)]
pub struct UpdateAccumulatorEvent {
    pub accumulator: Address,
}

#[contractevent(topics = ["config", "pool_template"])]
#[derive(Clone, Debug)]
pub struct UpdatePoolTemplateEvent {
    pub wasm_hash: soroban_sdk::BytesN<32>,
}

#[contractevent(topics = ["config", "position_limits"])]
#[derive(Clone, Debug)]
pub struct UpdatePositionLimitsEvent {
    pub max_supply_positions: u32,
    pub max_borrow_positions: u32,
}

#[contractevent(topics = ["config", "oracle_disabled"])]
#[derive(Clone, Debug)]
pub struct OracleDisabledEvent {
    pub asset: Address,
}

#[contractevent(topics = ["oracle", "twap_degraded"])]
#[derive(Clone, Debug)]
pub struct OracleTwapDegradedEvent {
    pub oracle: Address,
    pub reason_code: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use controller_interface::types::{
        AssetConfigRaw, EModeAssetConfig, MarketConfig, MarketOracleConfig, MarketStatus,
        OracleAssetRef, OraclePriceFluctuation, OracleReadMode, OracleSourceConfig,
        OracleSourceConfigOption, OracleStrategy, PositionMode, ReflectorBase,
        ReflectorSourceConfig,
    };
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{contract, vec, BytesN, Vec};

    #[contract]
    struct TestContract;

    fn setup() -> (Env, Address) {
        let env = Env::default();
        let contract = env.register(TestContract, ());
        (env, contract)
    }

    fn dummy_address(env: &Env) -> Address {
        Address::generate(env)
    }

    fn dummy_asset_config(env: &Env) -> AssetConfigRaw {
        AssetConfigRaw {
            loan_to_value_bps: 7500,
            liquidation_threshold_bps: 8000,
            liquidation_bonus_bps: 500,
            liquidation_fees_bps: 100,
            is_collateralizable: true,
            is_borrowable: true,
            e_mode_categories: soroban_sdk::Vec::new(env),
            is_isolated_asset: false,
            is_siloed_borrowing: false,
            is_flashloanable: true,
            isolation_borrow_enabled: false,
            isolation_debt_ceiling_usd_wad: 0,
            flashloan_fee_bps: 9,
            borrow_cap: 0,
            supply_cap: 0,
            min_collat_floor_usd_wad: crate::constants::MIN_DUST_FLOOR_WAD,
            min_debt_floor_usd_wad: crate::constants::MIN_DUST_FLOOR_WAD,
        }
    }

    fn dummy_market_config(env: &Env) -> MarketConfig {
        let asset = dummy_address(env);
        let oracle = dummy_address(env);
        MarketConfig {
            status: MarketStatus::Active,
            asset_config: dummy_asset_config(env),
            oracle_config: MarketOracleConfig {
                asset_decimals: 7,
                max_price_stale_seconds: 900,
                tolerance: OraclePriceFluctuation {
                    first_upper_ratio_bps: 10_200,
                    first_lower_ratio_bps: 9_800,
                    last_upper_ratio_bps: 10_500,
                    last_lower_ratio_bps: 9_500,
                },
                strategy: OracleStrategy::PrimaryWithAnchor,
                primary: OracleSourceConfig::Reflector(ReflectorSourceConfig {
                    contract: oracle.clone(),
                    asset: OracleAssetRef::Stellar(asset.clone()),
                    read_mode: OracleReadMode::Twap(12),
                    decimals: 14,
                    resolution_seconds: 300,
                    base: ReflectorBase::Usd,
                }),
                anchor: OracleSourceConfigOption::Some(OracleSourceConfig::Reflector(
                    ReflectorSourceConfig {
                        contract: oracle,
                        asset: OracleAssetRef::Stellar(asset),
                        read_mode: OracleReadMode::Spot,
                        decimals: 7,
                        resolution_seconds: 300,
                        base: ReflectorBase::Usd,
                    },
                )),
                min_sanity_price_wad: 0,
                max_sanity_price_wad: 0,
            },
        }
    }

    // ---------- enum derive exercisers ----------

    #[test]
    fn event_position_mode_eq_and_from() {
        assert_eq!(EventPositionMode::None, EventPositionMode::None);
        assert_ne!(EventPositionMode::Long, EventPositionMode::Short);
        assert_eq!(
            EventPositionMode::from(PositionMode::Normal),
            EventPositionMode::None
        );
        assert_eq!(
            EventPositionMode::from(PositionMode::Multiply),
            EventPositionMode::Multiply
        );
        assert_eq!(
            EventPositionMode::from(PositionMode::Long),
            EventPositionMode::Long
        );
        assert_eq!(
            EventPositionMode::from(PositionMode::Short),
            EventPositionMode::Short
        );
    }

    #[test]
    fn event_oracle_type_eq_and_from() {
        assert_eq!(EventOracleType::None, EventOracleType::None);
        assert_ne!(EventOracleType::None, EventOracleType::Normal);
    }

    #[test]
    fn event_pricing_method_eq_and_from() {
        assert_eq!(EventPricingMethod::None, EventPricingMethod::None);
        assert_ne!(EventPricingMethod::Safe, EventPricingMethod::Instant);
        assert_eq!(
            EventPricingMethod::from(OracleStrategy::Single),
            EventPricingMethod::Instant
        );
        assert_eq!(
            EventPricingMethod::from(OracleStrategy::PrimaryWithAnchor),
            EventPricingMethod::Mix
        );
    }

    // ---------- AccountMeta conversion ----------

    #[test]
    fn event_account_attributes_from_account_meta_isolated() {
        let env = Env::default();
        let owner = dummy_address(&env);
        let iso = dummy_address(&env);
        let meta = AccountMeta {
            owner: owner.clone(),
            is_isolated: true,
            e_mode_category_id: 0,
            mode: PositionMode::Normal,
            isolated_asset: Some(iso.clone()),
        };
        // Tuple order is wire ABI: [owner, e_mode, isolated, mode, isolated_token].
        let attrs = EventAccountAttributes::from(&meta);
        assert_eq!(attrs.0, owner);
        assert_eq!(attrs.1, 0);
        assert!(attrs.2);
        assert_eq!(attrs.3, EventPositionMode::None);
        assert_eq!(attrs.4, Some(iso));
    }

    #[test]
    fn event_account_attributes_from_account_meta_emode() {
        let env = Env::default();
        let owner = dummy_address(&env);
        let meta = AccountMeta {
            owner: owner.clone(),
            is_isolated: false,
            e_mode_category_id: 3,
            mode: PositionMode::Long,
            isolated_asset: None,
        };
        let attrs = EventAccountAttributes::from(&meta);
        assert_eq!(attrs.0, owner);
        assert_eq!(attrs.1, 3);
        assert!(!attrs.2);
        assert_eq!(attrs.3, EventPositionMode::Long);
        assert_eq!(attrs.4, None);
    }

    // ---------- EventOracleProvider::from_market ----------

    #[test]
    fn event_oracle_provider_from_market_builds_struct() {
        let env = Env::default();
        let market = dummy_market_config(&env);
        let asset = dummy_address(&env);
        let provider = EventOracleProvider::from_market(&env, &asset, &market);
        assert_eq!(
            provider.primary_provider,
            OracleProviderKind::ReflectorSep40 as u32
        );
        assert_eq!(provider.primary_decimals, 14);
        assert_eq!(provider.primary_twap_records, 12);
        assert_eq!(provider.primary_max_stale_seconds, 900);
        assert!(provider.primary_asset.is_some());
        assert_eq!(provider.anchor_decimals, 7);
        assert_eq!(provider.anchor_twap_records, 0);
        assert_eq!(provider.anchor_max_stale_seconds, 900);
        assert!(provider.anchor_contract.is_some());
    }

    #[test]
    fn update_asset_oracle_event_nests_oracle_fields_under_oracle_key() {
        extern crate std;
        use soroban_sdk::testutils::Events;
        use soroban_sdk::xdr::{ContractEventBody, ScVal};
        use std::string::{String, ToString};
        use std::vec::Vec as StdVec;

        fn map_keys(v: &ScVal) -> StdVec<String> {
            match v {
                ScVal::Map(Some(m)) => m
                    .iter()
                    .filter_map(|e| match &e.key {
                        ScVal::Symbol(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .collect(),
                _ => StdVec::new(),
            }
        }
        fn nested<'a>(v: &'a ScVal, key: &str) -> &'a ScVal {
            match v {
                ScVal::Map(Some(m)) => m
                    .iter()
                    .find(|e| matches!(&e.key, ScVal::Symbol(s) if s.to_string() == key))
                    .map(|e| &e.val)
                    .expect("nested key present"),
                _ => panic!("not a map"),
            }
        }

        let (env, contract) = setup();
        env.as_contract(&contract, || {
            let asset = dummy_address(&env);
            let market = dummy_market_config(&env);
            UpdateAssetOracleEvent {
                asset: asset.clone(),
                oracle: EventOracleProvider::from_market(&env, &asset, &market),
            }
            .publish(&env);
        });

        let all = env.events().all();
        let xdr_events = all.events();
        let last = xdr_events.last().expect("event published");
        let ContractEventBody::V0(body) = &last.body;
        let data = &body.data;

        // The event data exposes only the two struct fields at the top level;
        // sanity bounds and quote tokens are NOT top-level.
        let top = map_keys(data);
        assert!(top.iter().any(|k| k == "oracle"), "top keys: {:?}", top);
        assert!(top.iter().any(|k| k == "asset"));
        assert!(!top.iter().any(|k| k == "min_sanity_price_wad"));
        assert!(!top.iter().any(|k| k == "primary_quote_token"));

        // Sanity bounds and per-source quote tokens live inside `oracle`.
        let oracle_keys = map_keys(nested(data, "oracle"));
        for expected in [
            "min_sanity_price_wad",
            "max_sanity_price_wad",
            "primary_quote_token",
            "anchor_quote_token",
        ] {
            assert!(
                oracle_keys.iter().any(|k| k == expected),
                "missing {expected} in oracle keys: {oracle_keys:?}"
            );
        }
    }

    // ---------- emit_* helpers ----------

    #[test]
    fn emit_helpers_publish_without_panicking() {
        let (env, contract) = setup();
        env.as_contract(&contract, || {
            let asset = dummy_address(&env);
            let caller = dummy_address(&env);
            let market = dummy_market_config(&env);

            CreateMarketEvent {
                base_asset: asset.clone(),
                max_borrow_rate: 0,
                base_borrow_rate: 0,
                slope1: 0,
                slope2: 0,
                slope3: 0,
                mid_utilization: 0,
                optimal_utilization: 0,
                max_utilization: 0,
                reserve_factor: 0,
                market_address: asset.clone(),
                config: dummy_asset_config(&env),
            }
            .publish(&env);

            UpdateMarketParamsEvent {
                asset: asset.clone(),
                max_borrow_rate_ray: 0,
                base_borrow_rate_ray: 0,
                slope1_ray: 0,
                slope2_ray: 0,
                slope3_ray: 0,
                mid_utilization_ray: 0,
                optimal_utilization_ray: 0,
                max_utilization_ray: 0,
                reserve_factor_bps: 0,
            }
            .publish(&env);

            let mut market_updates = Vec::new(&env);
            market_updates.push_back(EventMarketState::from(&controller_interface::types::MarketStateSnapshot {
                asset: asset.clone(),
                timestamp: 0,
                supply_index_ray: 0,
                borrow_index_ray: 0,
                reserves_ray: 0,
                supplied_ray: 0,
                borrowed_ray: 0,
                revenue_ray: 0,
                asset_price_wad: None,
            }));
            UpdateMarketStateBatchEvent {
                updates: market_updates,
            }
            .publish(&env);

            let mut deposits = Vec::new(&env);
            deposits.push_back(EventDepositDelta(
                PositionAction::Supply,
                asset.clone(),
                0,
                0,
                0,
                0,
                0,
                0,
            ));
            UpdatePositionBatchEvent {
                account_id: 1,
                account_attributes: EventAccountAttributes(
                    caller.clone(),
                    0,
                    false,
                    EventPositionMode::None,
                    None,
                ),
                deposits,
                borrows: Vec::new(&env),
            }
            .publish(&env);

            FlashLoanEvent {
                asset: asset.clone(),
                receiver: caller.clone(),
                caller: caller.clone(),
                amount: 0,
                fee: 0,
            }
            .publish(&env);

            UpdateAssetConfigEvent {
                asset: asset.clone(),
                config: dummy_asset_config(&env),
            }
            .publish(&env);

            UpdateAssetOracleEvent {
                asset: asset.clone(),
                oracle: EventOracleProvider::from_market(&env, &asset, &market),
            }
            .publish(&env);

            UpdateEModeCategoryEvent {
                category: EventEModeCategory {
                    category_id: 1,
                    loan_to_value_bps: 9000,
                    liquidation_threshold_bps: 9500,
                    liquidation_bonus_bps: 200,
                    is_deprecated: false,
                },
            }
            .publish(&env);

            UpdateEModeAssetEvent {
                asset: asset.clone(),
                config: EModeAssetConfig {
                    is_collateralizable: true,
                    is_borrowable: true,
                },
                category_id: 1,
            }
            .publish(&env);

            RemoveEModeAssetEvent {
                asset: asset.clone(),
                category_id: 1,
            }
            .publish(&env);

            UpdateDebtCeilingBatchEvent {
                updates: Vec::new(&env),
            }
            .publish(&env);

            CleanBadDebtEvent {
                account_id: 1,
                total_borrow_usd_wad: 0,
                total_collateral_usd_wad: 0,
            }
            .publish(&env);

            InitialMultiplyPaymentEvent {
                token: asset.clone(),
                amount: 0,
                usd_value_wad: 0,
                account_id: 1,
            }
            .publish(&env);

            ApproveTokenEvent {
                wasm_hash: BytesN::from_array(&env, &[0u8; 32]),
                approved: true,
            }
            .publish(&env);

            // Reference vec! to keep it used even if the macro path changes.
            let _ignored: Vec<Address> = vec![&env];
        });
    }
}
