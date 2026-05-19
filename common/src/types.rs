use soroban_sdk::{contracttype, panic_with_error, Address, Env, Map, String, Symbol, Vec};

use crate::constants::{BPS, MAX_BORROW_RATE_RAY, RAY};
use crate::errors::CollateralError;

// Asset + amount pair.
pub type Payment = (Address, i128);



// Position discriminants.
pub const POSITION_TYPE_DEPOSIT: u32 = 1;
pub const POSITION_TYPE_BORROW: u32 = 2;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AccountPositionType {
    Deposit = 1,
    Borrow = 2,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PositionMode {
    Normal = 0,
    Multiply = 1,
    Long = 2,
    Short = 3,
}



#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketParams {
    pub max_borrow_rate_ray: i128,
    pub base_borrow_rate_ray: i128,
    pub slope1_ray: i128,
    pub slope2_ray: i128,
    pub slope3_ray: i128,
    // Max utilization.
    pub mid_utilization_ray: i128,
    pub optimal_utilization_ray: i128,
    pub max_utilization_ray: i128,
    // Reserve factor (bps).
    pub reserve_factor_bps: u32,
    pub asset_id: Address,
    pub asset_decimals: u32,
}

impl MarketParams {
    // Interest rate model view.
    pub fn rate_model_view(&self) -> InterestRateModel {
        InterestRateModel {
            max_borrow_rate_ray: self.max_borrow_rate_ray,
            base_borrow_rate_ray: self.base_borrow_rate_ray,
            slope1_ray: self.slope1_ray,
            slope2_ray: self.slope2_ray,
            slope3_ray: self.slope3_ray,
            mid_utilization_ray: self.mid_utilization_ray,
            optimal_utilization_ray: self.optimal_utilization_ray,
            max_utilization_ray: self.max_utilization_ray,
            reserve_factor_bps: self.reserve_factor_bps,
        }
    }

    // Validates the interest-rate model.
    pub fn verify_rate_model(&self, env: &Env) {
        self.rate_model_view().verify(env);
    }
}

// Interest rate model.
#[contracttype]
#[derive(Clone, Debug)]
pub struct InterestRateModel {
    pub max_borrow_rate_ray: i128,
    pub base_borrow_rate_ray: i128,
    pub slope1_ray: i128,
    pub slope2_ray: i128,
    pub slope3_ray: i128,
    pub mid_utilization_ray: i128,
    pub optimal_utilization_ray: i128,
    // Hard utilization ceiling.
    pub max_utilization_ray: i128,
    // Reserve factor (bps).
    pub reserve_factor_bps: u32,
}

impl InterestRateModel {
    // Validates rate-model invariants.
    pub fn verify(&self, env: &Env) {
        if self.base_borrow_rate_ray < 0
            || self.slope1_ray < self.base_borrow_rate_ray
            || self.slope2_ray < self.slope1_ray
            || self.slope3_ray < self.slope2_ray
            || self.max_borrow_rate_ray < self.slope3_ray
        {
            panic_with_error!(env, CollateralError::InvalidBorrowParams);
        }
        if self.max_borrow_rate_ray <= self.base_borrow_rate_ray {
            panic_with_error!(env, CollateralError::InvalidBorrowParams);
        }
        // Compound-interest Taylor envelope cap: per-chunk `x <= 2 RAY`.
        if self.max_borrow_rate_ray > MAX_BORROW_RATE_RAY {
            panic_with_error!(env, CollateralError::InvalidBorrowParams);
        }
        if self.mid_utilization_ray <= 0 {
            panic_with_error!(env, CollateralError::InvalidUtilRange);
        }
        if self.optimal_utilization_ray <= self.mid_utilization_ray {
            panic_with_error!(env, CollateralError::InvalidUtilRange);
        }
        if self.optimal_utilization_ray >= RAY {
            panic_with_error!(env, CollateralError::OptUtilTooHigh);
        }
        /// Validates max utilization is within bounds.
        if self.max_utilization_ray < self.optimal_utilization_ray || self.max_utilization_ray > RAY
        {
            panic_with_error!(env, CollateralError::InvalidUtilRange);
        }
        if i128::from(self.reserve_factor_bps) >= BPS {
            panic_with_error!(env, CollateralError::InvalidReserveFactor);
        }
    }
}



#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPosition {
    pub scaled_amount_ray: i128,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub liquidation_fees_bps: u32,
    pub loan_to_value_bps: u32,
}



#[contracttype]
#[derive(Clone, Debug)]
pub struct AssetConfig {
    // Risk parameters (bps).
    pub loan_to_value_bps: u32,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub liquidation_fees_bps: u32,
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub is_isolated_asset: bool,
    pub is_siloed_borrowing: bool,
    pub is_flashloanable: bool,
    pub isolation_borrow_enabled: bool,
    // Isolation debt ceiling (USD-WAD).
    pub isolation_debt_ceiling_usd_wad: i128,
    pub flashloan_fee_bps: u32,
    pub borrow_cap: i128,
    pub supply_cap: i128,
    pub min_collat_floor_usd_wad: i128,
    pub min_debt_floor_usd_wad: i128,
    // E-mode memberships.
    pub e_mode_categories: Vec<u32>,
}

impl AssetConfig {
    pub fn can_supply(&self) -> bool {
        self.is_collateralizable
    }

    pub fn can_borrow(&self) -> bool {
        self.is_borrowable
    }

    pub fn is_isolated(&self) -> bool {
        self.is_isolated_asset
    }

    pub fn is_siloed_borrowing(&self) -> bool {
        self.is_siloed_borrowing
    }

    pub fn can_borrow_in_isolation(&self) -> bool {
        self.isolation_borrow_enabled
    }

    pub fn has_emode(&self) -> bool {
        !self.e_mode_categories.is_empty()
    }
}



#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountAttributes {
    pub is_isolated: bool,
    pub e_mode_category_id: u32,
    pub mode: PositionMode,
}

impl AccountAttributes {
    pub fn has_emode(&self) -> bool {
        self.e_mode_category_id > 0
    }
}

// Account metadata.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountMeta {
    pub owner: Address,
    pub is_isolated: bool,
    pub e_mode_category_id: u32,
    pub mode: PositionMode,
    pub isolated_asset: Option<Address>,
}



// E-mode category config.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EModeCategory {
    pub loan_to_value_bps: u32,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub is_deprecated: bool,
    pub assets: Map<Address, EModeAssetConfig>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EModeAssetConfig {
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
}



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



#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketIndex {
    pub borrow_index_ray: i128,
    pub supply_index_ray: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketStateSnapshot {
    pub asset: Address,
    pub timestamp: u64,
    pub supply_index_ray: i128,
    pub borrow_index_ray: i128,
    pub reserves_ray: i128,
    pub supplied_ray: i128,
    pub borrowed_ray: i128,
    pub revenue_ray: i128,
    pub asset_price_wad: Option<i128>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketIndexView {
    pub asset: Address,
    pub supply_index_ray: i128,
    pub borrow_index_ray: i128,
    pub price_wad: i128,
    pub safe_price_wad: i128,
    pub aggregator_price_wad: i128,
    pub within_first_tolerance: bool,
    pub within_second_tolerance: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct AssetExtendedConfigView {
    pub asset: Address,
    pub pool_address: Address,
    pub price_wad: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolPositionMutation {
    pub position: AccountPosition,
    pub market_index: MarketIndex,
    pub market_state: MarketStateSnapshot,
    pub actual_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolStrategyMutation {
    pub position: AccountPosition,
    pub market_index: MarketIndex,
    pub market_state: MarketStateSnapshot,
    pub actual_amount: i128,
    pub amount_received: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolAmountMutation {
    pub market_state: MarketStateSnapshot,
    pub actual_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolSyncData {
    pub params: MarketParams,
    pub state: PoolState,
}



#[contracttype]
#[derive(Clone, Debug)]
pub struct PositionLimits {
    pub max_borrow_positions: u32,
    pub max_supply_positions: u32,
}



#[contracttype]
#[derive(Clone, Debug)]
pub struct PaymentTuple {
    pub asset: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct LiquidationEstimate {
    pub seized_collaterals: Vec<PaymentTuple>,
    pub protocol_fees: Vec<PaymentTuple>,
    pub refunds: Vec<PaymentTuple>,
    pub max_payment_wad: i128,
    pub bonus_rate_bps: i128,
}

// Seized collateral.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SeizeEntry {
    pub asset: Address,
    pub amount: i128,
    pub protocol_fee: i128,
    pub feed: PriceFeed,
    pub market_index: MarketIndex,
}

// Repaid debt.
#[contracttype]
#[derive(Clone, Debug)]
pub struct RepayEntry {
    pub asset: Address,
    pub amount: i128,
    pub usd_wad: i128,
    pub feed: PriceFeed,
    pub market_index: MarketIndex,
}

// Liquidation result.
#[derive(Clone)]
pub struct LiquidationResult {
    pub seized: Vec<SeizeEntry>,
    pub repaid: Vec<RepayEntry>,
    pub refunds: Vec<Payment>,
    pub max_debt_usd: i128,
    pub bonus_bps: i128,
}



// Swap venue.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapVenue {
    Soroswap,
    Aquarius,
    Phoenix,
    NativeAmm,
    StaticBridge,
}

// Swap hop.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapHop {
    // Fee (bps).
    pub fee_bps: u32,
    /// Pool contract address or venue-specific ID.
    pub pool: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub venue: SwapVenue,
}

// Swap path.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapPath {
    pub hops: Vec<SwapHop>,
    // Split (PPM).
    pub split_ppm: u32,
}

// Swap request.
#[contracttype]
#[derive(Clone, Debug)]
pub struct AggregatorSwap {
    // Convergence paths.
    pub paths: Vec<SwapPath>,
    // Slippage floor.
    pub total_min_out: i128,
}

// Backward-compatible name used by the Certora specs.
pub type SwapSteps = AggregatorSwap;

// Batch swap.
#[contracttype]
#[derive(Clone, Debug)]
pub struct BatchSwap {
    pub paths: Vec<SwapPath>,
    // Referral ID.
    pub referral_id: u64,
    pub sender: Address,
    pub total_in: i128,
    pub total_min_out: i128,
}



// Market status.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum MarketStatus {
    PendingOracle = 0,
    Active = 1,
    Disabled = 2,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketConfig {
    pub status: MarketStatus,
    pub asset_config: AssetConfig,
    pub pool_address: Address,
    pub oracle_config: MarketOracleConfig,
}

// Account state.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Account {
    pub owner: Address,
    pub is_isolated: bool,
    pub e_mode_category_id: u32,
    pub mode: PositionMode,
    pub isolated_asset: Option<Address>,
    pub supply_positions: Map<Address, AccountPosition>,
    pub borrow_positions: Map<Address, AccountPosition>,
}

impl Account {
    pub fn attributes(&self) -> AccountAttributes {
        AccountAttributes::from(self)
    }

    pub fn has_emode(&self) -> bool {
        self.e_mode_category_id > 0
    }

    pub fn try_isolated_token(&self) -> Option<Address> {
        self.isolated_asset.clone()
    }
}

impl From<&Account> for AccountAttributes {
    fn from(account: &Account) -> Self {
        AccountAttributes {
            is_isolated: account.is_isolated,
            e_mode_category_id: account.e_mode_category_id,
            mode: account.mode,
        }
    }
}

impl From<&AccountMeta> for AccountAttributes {
    fn from(account: &AccountMeta) -> Self {
        AccountAttributes {
            is_isolated: account.is_isolated,
            e_mode_category_id: account.e_mode_category_id,
            mode: account.mode,
        }
    }
}

// Storage keys.

// Controller storage keys.
#[contracttype]
#[derive(Clone, Debug)]
pub enum ControllerKey {
    // Instance-scoped
    PoolTemplate,
    Aggregator,
    Accumulator,
    AccountNonce,
    PositionLimits,
    LastEModeCategoryId,
    FlashLoanOngoing,

    // Persistent-scoped
    Market(Address),
    AccountMeta(u64),
    SupplyPositions(u64),
    BorrowPositions(u64),
    EModeCategory(u32),
    IsolatedDebt(Address),
    // Asset list.
    PoolsList,
}

// Pool storage keys.
#[contracttype]
#[derive(Clone, Debug)]
pub enum PoolKey {
    Params,
    State,
}

// Pool state.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolState {
    pub supplied_ray: i128,
    pub borrowed_ray: i128,
    pub revenue_ray: i128,
    pub borrow_index_ray: i128,
    pub supply_index_ray: i128,
    pub last_timestamp: u64,
}
