use soroban_sdk::{contractevent, contracttype, Address, Env, Symbol};

use crate::types::{
    Account, AccountMeta, AccountPosition, AccountPositionType, AssetConfig, EModeAssetConfig,
    EModeCategory, ExchangeSource, MarketConfig, OraclePriceFluctuation, OracleType, PositionMode,
};

// ---------------------------------------------------------------------------
// Event data structs
// ---------------------------------------------------------------------------

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

impl From<OracleType> for EventOracleType {
    fn from(value: OracleType) -> Self {
        match value {
            OracleType::None => Self::None,
            OracleType::Normal => Self::Normal,
        }
    }
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

impl From<ExchangeSource> for EventPricingMethod {
    fn from(value: ExchangeSource) -> Self {
        match value {
            ExchangeSource::SpotOnly => Self::Instant,
            ExchangeSource::SpotVsTwap => Self::Safe,
            ExchangeSource::DualOracle => Self::Mix,
        }
    }
}

/// Indexer-facing position payload. Carries the side, asset, and account
/// id alongside the same risk-param snapshot held in
/// [`AccountPosition`]. Bps fields use `u32` to match the storage width
/// and decode as JS `number` on the indexer side.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventAccountPosition {
    pub position_type: EventAccountPositionType,
    pub asset_id: Address,
    pub scaled_amount_ray: i128,
    pub account_nonce: u64,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub liquidation_fees_bps: u32,
    pub loan_to_value_bps: u32,
}

impl EventAccountPosition {
    /// Compose the event payload from the stored value plus the
    /// emit-site context (side, asset, account id).
    pub fn new(
        side: AccountPositionType,
        asset: Address,
        account_id: u64,
        position: &AccountPosition,
    ) -> Self {
        Self {
            position_type: side.into(),
            asset_id: asset,
            scaled_amount_ray: position.scaled_amount_ray,
            account_nonce: account_id,
            liquidation_threshold_bps: position.liquidation_threshold_bps,
            liquidation_bonus_bps: position.liquidation_bonus_bps,
            liquidation_fees_bps: position.liquidation_fees_bps,
            loan_to_value_bps: position.loan_to_value_bps,
        }
    }
}

/// Indexer-facing snapshot of [`AccountMeta`]. `owner` is the canonical
/// account owner — indexers use this as the authoritative source for
/// the position doc's `address` field, since the event-level `caller`
/// can be the controller itself on strategy-internal supply legs.
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
    pub oracle_contract_address: Option<Address>,
    pub pricing_method: EventPricingMethod,
    pub oracle_type: EventOracleType,
    pub exchange_source: u32,
    pub asset_decimals: u32,
    pub onedex_pair_id: u32,
    pub max_price_stale_seconds: u64,
    pub reflector_cex_oracle: Option<Address>,
    pub reflector_cex_asset_kind: u32,
    pub reflector_cex_symbol: Option<Symbol>,
    pub reflector_cex_decimals: u32,
    pub reflector_dex_oracle: Option<Address>,
    pub reflector_dex_asset_kind: u32,
    pub reflector_dex_decimals: u32,
    pub reflector_twap_records: u32,
}

impl EventOracleProvider {
    pub fn from_market(env: &Env, market: &MarketConfig) -> Self {
        Self {
            base_token_id: market.oracle_config.base_asset.clone(),
            quote_token_id: Symbol::new(env, "USD"),
            tolerance: market.oracle_config.tolerance.clone(),
            oracle_contract_address: market.cex_oracle.clone(),
            pricing_method: market.oracle_config.exchange_source.into(),
            oracle_type: market.oracle_config.oracle_type.into(),
            exchange_source: market.oracle_config.exchange_source as u32,
            asset_decimals: market.oracle_config.asset_decimals,
            onedex_pair_id: 0,
            max_price_stale_seconds: market.oracle_config.max_price_stale_seconds,
            reflector_cex_oracle: market.cex_oracle.clone(),
            reflector_cex_asset_kind: market.cex_asset_kind.clone() as u32,
            reflector_cex_symbol: market
                .cex_oracle
                .as_ref()
                .map(|_| market.cex_symbol.clone()),
            reflector_cex_decimals: market.cex_decimals,
            reflector_dex_oracle: market.dex_oracle.clone(),
            reflector_dex_asset_kind: market.dex_asset_kind.clone() as u32,
            reflector_dex_decimals: market.dex_decimals,
            reflector_twap_records: market.twap_records,
        }
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
    pub config: AssetConfig,
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

#[contractevent(topics = ["market", "state_update"])]
#[derive(Clone, Debug)]
pub struct UpdateMarketStateEvent {
    pub asset: Address,
    pub timestamp: u64,
    pub supply_index_ray: i128,
    pub borrow_index_ray: i128,
    pub reserves_ray: i128,
    pub supplied_ray: i128,
    pub borrowed_ray: i128,
    pub revenue_ray: i128,
    pub asset_price_wad: i128,
}

#[contractevent(topics = ["position", "update"])]
#[derive(Clone, Debug)]
pub struct UpdatePositionEvent {
    /// Discriminator for the controller flow that produced this event.
    /// Balance-mutating entrypoints share the `["position","update"]` topic;
    /// indexers use this field to distinguish actions. Values are lowercase
    /// symbols of at most nine bytes so they fit in `Symbol::short`.
    ///
    ///   Plain flows:
    ///     - `supply`     - supply position update
    ///     - `borrow`     - borrow position update
    ///     - `withdraw`   - withdrawal position update
    ///     - `repay`      - repayment position update
    ///
    ///   Admin / aggregated:
    ///     - `param_upd`  - keeper risk-parameter propagation
    ///     - `multiply`   - strategy borrow leg
    ///
    ///   Liquidation:
    ///     - `liq_repay`  - liquidator repays debtor debt
    ///     - `liq_seize`  - liquidator seizes debtor collateral
    ///
    ///   Strategy flows:
    ///     - `sw_debt_r`  - `process_swap_debt`   (repay leg, source debt)
    ///     - `sw_col_wd`  - `process_swap_collateral` (withdraw leg)
    ///     - `rp_col_wd`  - `process_repay_debt_with_collateral` (withdraw leg)
    ///     - `rp_col_r`   - `process_repay_debt_with_collateral` (repay leg)
    ///     - `close_wd`   - `execute_withdraw_all` (close-position leg)
    pub action: Symbol,
    pub index: i128,
    pub amount: i128,
    pub position: EventAccountPosition,
    pub asset_price: Option<i128>,
    pub caller: Option<Address>,
    pub account_attributes: Option<EventAccountAttributes>,
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
    pub config: AssetConfig,
}

#[contractevent(topics = ["config", "oracle"])]
#[derive(Clone, Debug)]
pub struct UpdateAssetOracleEvent {
    pub asset: Address,
    pub oracle: EventOracleProvider,
}

/// Indexer-facing category payload — carries the `category_id`
/// discriminant alongside the params held in [`EModeCategory`]. The
/// member-asset map is omitted; per-asset memberships are emitted via
/// [`UpdateEModeAssetEvent`] / [`RemoveEModeAssetEvent`].
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
    pub fn new(category_id: u32, category: &EModeCategory) -> Self {
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

#[contractevent(topics = ["pool", "insolvent"])]
#[derive(Clone, Debug)]
pub struct PoolInsolventEvent {
    pub asset: Address,
    pub bad_debt_ratio_bps: i128,
    pub old_supply_index_ray: i128,
    pub new_supply_index_ray: i128,
}

#[contractevent(topics = ["config", "approve_token_wasm"])]
#[derive(Clone, Debug)]
pub struct ApproveTokenWasmEvent {
    pub wasm_hash: soroban_sdk::BytesN<32>,
    pub approved: bool,
}

// ---------------------------------------------------------------------------
// Event emission helpers
// ---------------------------------------------------------------------------

pub fn emit_create_market(env: &Env, event: CreateMarketEvent) {
    event.publish(env);
}

pub fn emit_update_market_params(env: &Env, event: UpdateMarketParamsEvent) {
    event.publish(env);
}

pub fn emit_update_market_state(env: &Env, event: UpdateMarketStateEvent) {
    event.publish(env);
}

pub fn emit_update_position(env: &Env, event: UpdatePositionEvent) {
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

pub fn emit_clean_bad_debt(env: &Env, event: CleanBadDebtEvent) {
    event.publish(env);
}

pub fn emit_initial_multiply_payment(env: &Env, event: InitialMultiplyPaymentEvent) {
    event.publish(env);
}

pub fn emit_pool_insolvent(env: &Env, event: PoolInsolventEvent) {
    event.publish(env);
}

pub fn emit_approve_token_wasm(env: &Env, event: ApproveTokenWasmEvent) {
    event.publish(env);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AssetConfig, EModeAssetConfig, ExchangeSource, MarketConfig, MarketStatus,
        OracleProviderConfig, OracleType, PositionMode, ReflectorAssetKind,
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

    fn dummy_asset_config(env: &Env) -> AssetConfig {
        AssetConfig {
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
        }
    }

    fn dummy_market_config(env: &Env) -> MarketConfig {
        MarketConfig {
            status: MarketStatus::Active,
            asset_config: dummy_asset_config(env),
            pool_address: dummy_address(env),
            oracle_config: OracleProviderConfig::default_for(dummy_address(env), 7),
            cex_oracle: Some(dummy_address(env)),
            cex_asset_kind: ReflectorAssetKind::Other,
            cex_symbol: Symbol::new(env, "USD"),
            cex_decimals: 14,
            dex_oracle: None,
            dex_asset_kind: ReflectorAssetKind::Stellar,
            dex_symbol: Symbol::new(env, ""),
            dex_decimals: 7,
            twap_records: 12,
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
        assert_eq!(
            EventOracleType::from(OracleType::None),
            EventOracleType::None
        );
        assert_eq!(
            EventOracleType::from(OracleType::Normal),
            EventOracleType::Normal
        );
    }

    #[test]
    fn event_pricing_method_eq_and_from() {
        assert_eq!(EventPricingMethod::None, EventPricingMethod::None);
        assert_ne!(EventPricingMethod::Safe, EventPricingMethod::Instant);
        assert_eq!(
            EventPricingMethod::from(ExchangeSource::SpotOnly),
            EventPricingMethod::Instant
        );
        assert_eq!(
            EventPricingMethod::from(ExchangeSource::SpotVsTwap),
            EventPricingMethod::Safe
        );
        assert_eq!(
            EventPricingMethod::from(ExchangeSource::DualOracle),
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
        let provider = EventOracleProvider::from_market(&env, &market);
        assert_eq!(provider.reflector_cex_decimals, 14);
        assert_eq!(provider.reflector_dex_decimals, 7);
        assert_eq!(provider.reflector_twap_records, 12);
        assert!(provider.reflector_cex_symbol.is_some());
        assert!(provider.reflector_dex_oracle.is_none());
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

            emit_update_market_state(
                &env,
                UpdateMarketStateEvent {
                    asset: asset.clone(),
                    timestamp: 0,
                    supply_index_ray: 0,
                    borrow_index_ray: 0,
                    reserves_ray: 0,
                    supplied_ray: 0,
                    borrowed_ray: 0,
                    revenue_ray: 0,
                    asset_price_wad: 0,
                },
            );

            let pos = EventAccountPosition {
                position_type: EventAccountPositionType::Deposit,
                asset_id: asset.clone(),
                scaled_amount_ray: 0,
                account_nonce: 1,
                liquidation_threshold_bps: 0,
                liquidation_bonus_bps: 0,
                liquidation_fees_bps: 0,
                loan_to_value_bps: 0,
            };
            emit_update_position(
                &env,
                UpdatePositionEvent {
                    action: soroban_sdk::symbol_short!("supply"),
                    index: 0,
                    amount: 0,
                    position: pos,
                    asset_price: None,
                    caller: Some(caller.clone()),
                    account_attributes: None,
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
                    oracle: EventOracleProvider::from_market(&env, &market),
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

            emit_pool_insolvent(
                &env,
                PoolInsolventEvent {
                    asset: asset.clone(),
                    bad_debt_ratio_bps: 0,
                    old_supply_index_ray: 0,
                    new_supply_index_ray: 0,
                },
            );

            emit_approve_token_wasm(
                &env,
                ApproveTokenWasmEvent {
                    wasm_hash: BytesN::from_array(&env, &[0u8; 32]),
                    approved: true,
                },
            );

            // Reference vec! to keep it used even if the macro path changes.
            let _ignored: Vec<Address> = vec![&env];
        });
    }
}
