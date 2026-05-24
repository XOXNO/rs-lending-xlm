use soroban_sdk::{contractevent, contracttype, symbol_short, Address, Env, String, Symbol, Vec};

use crate::types::{
    Account, AccountMeta, AccountPosition, AccountPositionType, AssetConfigRaw, DebtPosition,
    EModeAssetConfig, EModeCategoryRaw, MarketConfig, OracleAssetRef, OraclePriceFluctuation,
    OracleProviderKind, OracleReadMode, OracleSourceConfig, OracleStrategy, PositionMode,
};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum EventAccountPositionType {
    None = 0,
    Deposit = 1,
    Borrow = 2,
}

impl From<AccountPositionType> for EventAccountPositionType {
    fn from(value: AccountPositionType) -> Self {
        match value {
            AccountPositionType::Deposit => Self::Deposit,
            AccountPositionType::Borrow => Self::Borrow,
        }
    }
}

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

// Position snapshot.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventAccountPosition {
    pub position_type: EventAccountPositionType,
    pub asset_id: Address,
    pub scaled_amount_ray: i128,
    pub account_nonce: u64,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub loan_to_value_bps: u32,
}

impl EventAccountPosition {
    // Creates event payload.
    pub fn new(
        side: AccountPositionType,
        asset: Address,
        account_id: u64,
        position: &AccountPosition,
    ) -> Self {
        Self {
            position_type: side.into(),
            asset_id: asset,
            scaled_amount_ray: position.scaled_amount.raw(),
            account_nonce: account_id,
            liquidation_threshold_bps: position.liquidation_threshold.raw() as u32,
            liquidation_bonus_bps: position.liquidation_bonus.raw() as u32,
            loan_to_value_bps: position.loan_to_value.raw() as u32,
        }
    }
}

// Account attributes snapshot.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventAccountAttributes {
    pub owner: Address,
    pub is_isolated_position: bool,
    pub e_mode_category_id: u32,
    pub mode: EventPositionMode,
    pub isolated_token: Option<Address>,
}

impl From<&Account> for EventAccountAttributes {
    fn from(value: &Account) -> Self {
        Self {
            owner: value.owner.clone(),
            is_isolated_position: value.is_isolated,
            e_mode_category_id: value.e_mode_category_id,
            mode: value.mode.into(),
            isolated_token: value.isolated_asset.clone(),
        }
    }
}

impl From<&AccountMeta> for EventAccountAttributes {
    fn from(value: &AccountMeta) -> Self {
        Self {
            owner: value.owner.clone(),
            is_isolated_position: value.is_isolated,
            e_mode_category_id: value.e_mode_category_id,
            mode: value.mode.into(),
            isolated_token: value.isolated_asset.clone(),
        }
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
    pub anchor_read_mode: u32,
    pub anchor_twap_records: u32,
    pub anchor_decimals: u32,
    pub anchor_resolution_seconds: u32,
    pub anchor_max_stale_seconds: u64,
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
            anchor_read_mode: anchor.as_ref().map(|source| source.read_mode).unwrap_or(0),
            anchor_twap_records: anchor
                .as_ref()
                .map(|source| source.twap_records)
                .unwrap_or(0),
            anchor_decimals: anchor.as_ref().map(|source| source.decimals).unwrap_or(0),
            anchor_resolution_seconds: anchor
                .as_ref()
                .map(|source| source.resolution_seconds)
                .unwrap_or(0),
            anchor_max_stale_seconds: anchor
                .as_ref()
                .map(|source| source.max_stale_seconds)
                .unwrap_or(0),
        }
    }
}

struct EventOracleSource {
    provider: u32,
    contract: Address,
    asset: Option<Address>,
    symbol: Option<Symbol>,
    feed_id: Option<String>,
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
                Self {
                    provider: OracleProviderKind::ReflectorSep40 as u32,
                    contract: config.contract.clone(),
                    asset,
                    symbol,
                    feed_id,
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
    pub reserve_factor_bps: u32,
}

#[contractevent(topics = ["market", "batch_state_update"])]
#[derive(Clone, Debug)]
pub struct UpdateMarketStateBatchEvent {
    pub updates: Vec<crate::types::MarketStateSnapshot>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EventPositionDelta {
    /// Action that produced this delta (e.g., supply, borrow, liq_repay).
    pub action: Symbol,
    pub position_type: AccountPositionType,
    pub asset: Address,
    pub scaled_amount_ray: i128,
    pub index_ray: i128,
    pub amount: i128,
    pub asset_price_wad: Option<i128>,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub loan_to_value_bps: u32,
}

impl EventPositionDelta {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        action: Symbol,
        position_type: AccountPositionType,
        asset: Address,
        index_ray: i128,
        amount: i128,
        position: &AccountPosition,
        asset_price_wad: Option<i128>,
    ) -> Self {
        Self {
            action,
            position_type,
            asset,
            scaled_amount_ray: position.scaled_amount.raw(),
            index_ray,
            amount,
            asset_price_wad,
            liquidation_threshold_bps: position.liquidation_threshold.raw() as u32,
            liquidation_bonus_bps: position.liquidation_bonus.raw() as u32,
            loan_to_value_bps: position.loan_to_value.raw() as u32,
        }
    }

    // Debt-position delta. Debt positions carry no collateral risk params, so
    // the risk fields are zeroed. CONSUMERS MUST gate interpretation of the
    // risk fields on `position_type == Borrow` (a zero here means "N/A", not a
    // configured 0% value).
    pub fn new_debt(
        action: Symbol,
        asset: Address,
        index_ray: i128,
        amount: i128,
        position: &DebtPosition,
        asset_price_wad: Option<i128>,
    ) -> Self {
        Self {
            action,
            position_type: AccountPositionType::Borrow,
            asset,
            scaled_amount_ray: position.scaled_amount.raw(),
            index_ray,
            amount,
            asset_price_wad,
            liquidation_threshold_bps: 0,
            liquidation_bonus_bps: 0,
            loan_to_value_bps: 0,
        }
    }
}

#[contractevent(topics = ["position", "batch_update"])]
#[derive(Clone, Debug)]
pub struct UpdatePositionBatchEvent {
    pub account_id: u64,
    pub account_attributes: EventAccountAttributes,
    pub updates: Vec<EventPositionDelta>,
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

// E-mode category snapshot.
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

#[contractevent(topics = ["debt", "ceiling_update"])]
#[derive(Clone, Debug)]
pub struct UpdateDebtCeilingEvent {
    pub asset: Address,
    pub total_debt_usd_wad: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EventDebtCeilingEntry {
    pub asset: Address,
    pub total_debt_usd_wad: i128,
}

#[contractevent(topics = ["debt", "ceiling_batch_update"])]
#[derive(Clone, Debug)]
pub struct UpdateDebtCeilingBatchEvent {
    pub updates: Vec<EventDebtCeilingEntry>,
}

#[contractevent(topics = ["debt", "bad_debt"])]
#[derive(Clone, Debug)]
pub struct CleanBadDebtEvent {
    pub account_id: u64,
    pub total_borrow_usd_wad: i128,
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

pub fn emit_create_market(env: &Env, event: CreateMarketEvent) {
    event.publish(env);
}

pub fn emit_update_market_params(env: &Env, event: UpdateMarketParamsEvent) {
    event.publish(env);
}

pub fn emit_update_market_state_batch(env: &Env, event: UpdateMarketStateBatchEvent) {
    event.publish(env);
}

pub fn emit_update_position_batch(env: &Env, event: UpdatePositionBatchEvent) {
    event.publish(env);
}

pub fn emit_flash_loan(env: &Env, event: FlashLoanEvent) {
    event.publish(env);
}

pub fn emit_update_asset_config(env: &Env, event: UpdateAssetConfigEvent) {
    event.publish(env);
}

pub fn emit_update_asset_oracle(env: &Env, event: UpdateAssetOracleEvent) {
    event.publish(env);
}

pub fn emit_update_emode_category(env: &Env, event: UpdateEModeCategoryEvent) {
    event.publish(env);
}

pub fn emit_update_emode_asset(env: &Env, event: UpdateEModeAssetEvent) {
    event.publish(env);
}

pub fn emit_remove_emode_asset(env: &Env, event: RemoveEModeAssetEvent) {
    event.publish(env);
}

pub fn emit_update_debt_ceiling(env: &Env, event: UpdateDebtCeilingEvent) {
    event.publish(env);
}

pub fn emit_update_debt_ceiling_batch(env: &Env, event: UpdateDebtCeilingBatchEvent) {
    event.publish(env);
}

pub fn emit_update_aggregator(env: &Env, event: UpdateAggregatorEvent) {
    event.publish(env);
}

pub fn emit_update_accumulator(env: &Env, event: UpdateAccumulatorEvent) {
    event.publish(env);
}

pub fn emit_update_pool_template(env: &Env, event: UpdatePoolTemplateEvent) {
    event.publish(env);
}

pub fn emit_update_position_limits(env: &Env, event: UpdatePositionLimitsEvent) {
    event.publish(env);
}

pub fn emit_oracle_disabled(env: &Env, event: OracleDisabledEvent) {
    event.publish(env);
}

pub fn emit_oracle_twap_degraded(env: &Env, event: OracleTwapDegradedEvent) {
    event.publish(env);
}

pub fn emit_clean_bad_debt(env: &Env, event: CleanBadDebtEvent) {
    event.publish(env);
}

pub fn emit_initial_multiply_payment(env: &Env, event: InitialMultiplyPaymentEvent) {
    event.publish(env);
}

pub fn emit_approve_token(env: &Env, event: ApproveTokenEvent) {
    event.publish(env);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AssetConfigRaw, EModeAssetConfig, MarketConfig, MarketOracleConfig, MarketStatus,
        OracleAssetRef, OraclePriceFluctuation, OracleReadMode, OracleSourceConfig,
        OracleSourceConfigOption, OracleStrategy, PositionMode, ReflectorSourceConfig,
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
            pool_address: dummy_address(env),
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
                }),
                anchor: OracleSourceConfigOption::Some(OracleSourceConfig::Reflector(
                    ReflectorSourceConfig {
                        contract: oracle,
                        asset: OracleAssetRef::Stellar(asset),
                        read_mode: OracleReadMode::Spot,
                        decimals: 7,
                        resolution_seconds: 300,
                    },
                )),
                min_sanity_price_wad: 0,
                max_sanity_price_wad: 0,
            },
        }
    }

    // ---------- enum derive exercisers ----------

    #[test]
    fn event_account_position_type_eq() {
        assert_eq!(
            EventAccountPositionType::None,
            EventAccountPositionType::None
        );
        assert_ne!(
            EventAccountPositionType::Deposit,
            EventAccountPositionType::Borrow
        );
        // From branches.
        assert_eq!(
            EventAccountPositionType::from(AccountPositionType::Deposit),
            EventAccountPositionType::Deposit
        );
        assert_eq!(
            EventAccountPositionType::from(AccountPositionType::Borrow),
            EventAccountPositionType::Borrow
        );
    }

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
        let attrs = EventAccountAttributes::from(&meta);
        assert_eq!(attrs.owner, owner);
        assert!(attrs.is_isolated_position);
        assert_eq!(attrs.e_mode_category_id, 0);
        assert_eq!(attrs.mode, EventPositionMode::None);
        assert_eq!(attrs.isolated_token, Some(iso));
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
        assert_eq!(attrs.owner, owner);
        assert!(!attrs.is_isolated_position);
        assert_eq!(attrs.e_mode_category_id, 3);
        assert_eq!(attrs.mode, EventPositionMode::Long);
        assert_eq!(attrs.isolated_token, None);
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

    // ---------- emit_* helpers ----------

    #[test]
    fn emit_helpers_publish_without_panicking() {
        let (env, contract) = setup();
        env.as_contract(&contract, || {
            let asset = dummy_address(&env);
            let caller = dummy_address(&env);
            let market = dummy_market_config(&env);

            emit_create_market(
                &env,
                CreateMarketEvent {
                    base_asset: asset.clone(),
                    max_borrow_rate: 0,
                    base_borrow_rate: 0,
                    slope1: 0,
                    slope2: 0,
                    slope3: 0,
                    mid_utilization: 0,
                    optimal_utilization: 0,
                    reserve_factor: 0,
                    market_address: asset.clone(),
                    config: dummy_asset_config(&env),
                },
            );

            emit_update_market_params(
                &env,
                UpdateMarketParamsEvent {
                    asset: asset.clone(),
                    max_borrow_rate_ray: 0,
                    base_borrow_rate_ray: 0,
                    slope1_ray: 0,
                    slope2_ray: 0,
                    slope3_ray: 0,
                    mid_utilization_ray: 0,
                    optimal_utilization_ray: 0,
                    reserve_factor_bps: 0,
                },
            );

            let mut market_updates = Vec::new(&env);
            market_updates.push_back(crate::types::MarketStateSnapshot {
                asset: asset.clone(),
                timestamp: 0,
                supply_index_ray: 0,
                borrow_index_ray: 0,
                reserves_ray: 0,
                supplied_ray: 0,
                borrowed_ray: 0,
                revenue_ray: 0,
                asset_price_wad: None,
            });
            emit_update_market_state_batch(
                &env,
                UpdateMarketStateBatchEvent {
                    updates: market_updates,
                },
            );

            let mut position_updates = Vec::new(&env);
            position_updates.push_back(EventPositionDelta {
                action: soroban_sdk::symbol_short!("supply"),
                position_type: AccountPositionType::Deposit,
                asset: asset.clone(),
                scaled_amount_ray: 0,
                index_ray: 0,
                amount: 0,
                asset_price_wad: None,
                liquidation_threshold_bps: 0,
                liquidation_bonus_bps: 0,
                loan_to_value_bps: 0,
            });
            emit_update_position_batch(
                &env,
                UpdatePositionBatchEvent {
                    account_id: 1,
                    account_attributes: EventAccountAttributes {
                        owner: caller.clone(),
                        is_isolated_position: false,
                        e_mode_category_id: 0,
                        mode: EventPositionMode::None,
                        isolated_token: None,
                    },
                    updates: position_updates,
                },
            );

            emit_flash_loan(
                &env,
                FlashLoanEvent {
                    asset: asset.clone(),
                    receiver: caller.clone(),
                    caller: caller.clone(),
                    amount: 0,
                    fee: 0,
                },
            );

            emit_update_asset_config(
                &env,
                UpdateAssetConfigEvent {
                    asset: asset.clone(),
                    config: dummy_asset_config(&env),
                },
            );

            emit_update_asset_oracle(
                &env,
                UpdateAssetOracleEvent {
                    asset: asset.clone(),
                    oracle: EventOracleProvider::from_market(&env, &asset, &market),
                },
            );

            emit_update_emode_category(
                &env,
                UpdateEModeCategoryEvent {
                    category: EventEModeCategory {
                        category_id: 1,
                        loan_to_value_bps: 9000,
                        liquidation_threshold_bps: 9500,
                        liquidation_bonus_bps: 200,
                        is_deprecated: false,
                    },
                },
            );

            emit_update_emode_asset(
                &env,
                UpdateEModeAssetEvent {
                    asset: asset.clone(),
                    config: EModeAssetConfig {
                        is_collateralizable: true,
                        is_borrowable: true,
                    },
                    category_id: 1,
                },
            );

            emit_remove_emode_asset(
                &env,
                RemoveEModeAssetEvent {
                    asset: asset.clone(),
                    category_id: 1,
                },
            );

            emit_update_debt_ceiling(
                &env,
                UpdateDebtCeilingEvent {
                    asset: asset.clone(),
                    total_debt_usd_wad: 0,
                },
            );

            emit_clean_bad_debt(
                &env,
                CleanBadDebtEvent {
                    account_id: 1,
                    total_borrow_usd_wad: 0,
                    total_collateral_usd_wad: 0,
                },
            );

            emit_initial_multiply_payment(
                &env,
                InitialMultiplyPaymentEvent {
                    token: asset.clone(),
                    amount: 0,
                    usd_value_wad: 0,
                    account_id: 1,
                },
            );

            emit_approve_token(
                &env,
                ApproveTokenEvent {
                    wasm_hash: BytesN::from_array(&env, &[0u8; 32]),
                    approved: true,
                },
            );

            // Reference vec! to keep it used even if the macro path changes.
            let _ignored: Vec<Address> = vec![&env];
        });
    }
}
