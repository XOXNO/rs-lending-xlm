use soroban_sdk::{contracttype, Address, Map, Symbol, Vec};

/// Internal asset + amount pair used by controller operation helpers.
/// Public contract entrypoints spell this as `(Address, i128)` so the Soroban
/// spec generator emits a tuple type instead of an undefined Rust alias.
pub type Payment = (Address, i128);

// ---------------------------------------------------------------------------
// Position types
// ---------------------------------------------------------------------------

// Position discriminants used inside composite storage keys. Stored as u32
// because `#[contracttype]` enum variant data does not support u8.
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum OracleType {
    None = 0,
    Normal = 1,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ExchangeSource {
    SpotOnly = 0,
    SpotVsTwap = 1,
    DualOracle = 3,
}

// ---------------------------------------------------------------------------
// Market parameters
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketParams {
    pub max_borrow_rate_ray: i128,
    pub base_borrow_rate_ray: i128,
    pub slope1_ray: i128,
    pub slope2_ray: i128,
    pub slope3_ray: i128,
    pub mid_utilization_ray: i128,
    pub optimal_utilization_ray: i128,
    pub reserve_factor_bps: i128,
    pub asset_id: Address,
    pub asset_decimals: u32,
}

/// Interest-rate model update payload. Separates the 8 mutable rate params
/// from `asset_id`/`asset_decimals`, which the controller resolves from
/// storage and never accepts from the caller.
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
    pub reserve_factor_bps: i128,
}

// ---------------------------------------------------------------------------
// Account position
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPosition {
    pub position_type: AccountPositionType,
    pub asset: Address,
    pub scaled_amount_ray: i128,
    pub account_id: u64,
    pub liquidation_threshold_bps: i128,
    pub liquidation_bonus_bps: i128,
    pub liquidation_fees_bps: i128,
    pub loan_to_value_bps: i128,
}

// ---------------------------------------------------------------------------
// Asset configuration
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug)]
pub struct AssetConfig {
    pub loan_to_value_bps: i128,
    pub liquidation_threshold_bps: i128,
    pub liquidation_bonus_bps: i128,
    pub liquidation_fees_bps: i128,
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub e_mode_enabled: bool,
    pub is_isolated_asset: bool,
    pub is_siloed_borrowing: bool,
    pub is_flashloanable: bool,
    pub isolation_borrow_enabled: bool,
    pub isolation_debt_ceiling_usd_wad: i128,
    pub flashloan_fee_bps: i128,
    pub borrow_cap: i128,
    pub supply_cap: i128,
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
        self.e_mode_enabled
    }
}

// ---------------------------------------------------------------------------
// Account attributes
// ---------------------------------------------------------------------------

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

/// Slim account context. The supply and borrow position maps live under
/// `ControllerKey::SupplyPositions` / `ControllerKey::BorrowPositions` and
/// serve as their own asset index — no Vec is maintained here.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountMeta {
    pub owner: Address,
    pub is_isolated: bool,
    pub e_mode_category_id: u32,
    pub mode: PositionMode,
    pub isolated_asset: Option<Address>,
}

// ---------------------------------------------------------------------------
// E-Mode
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug)]
pub struct EModeCategory {
    pub category_id: u32,
    pub loan_to_value_bps: i128,
    pub liquidation_threshold_bps: i128,
    pub liquidation_bonus_bps: i128,
    pub is_deprecated: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EModeAssetConfig {
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
}

// ---------------------------------------------------------------------------
// Reflector oracle config enums
// ---------------------------------------------------------------------------

/// SEP-40 asset variant selector for Reflector oracle calls.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReflectorAssetKind {
    Stellar = 0,
    Other = 1,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ReflectorConfig {
    pub cex_oracle: Address,
    pub cex_asset_kind: ReflectorAssetKind,
    pub cex_symbol: Symbol,
    pub cex_decimals: u32,
    pub dex_oracle: Option<Address>,
    pub dex_asset_kind: ReflectorAssetKind,
    pub dex_decimals: u32,
    pub twap_records: u32,
}

// ---------------------------------------------------------------------------
// Oracle
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug)]
pub struct OraclePriceFluctuation {
    pub first_upper_ratio_bps: i128,
    pub first_lower_ratio_bps: i128,
    pub last_upper_ratio_bps: i128,
    pub last_lower_ratio_bps: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct OracleProviderConfig {
    pub base_asset: Address,
    pub oracle_type: OracleType,
    pub exchange_source: ExchangeSource,
    pub asset_decimals: u32,
    pub tolerance: OraclePriceFluctuation,
    pub max_price_stale_seconds: u64,
}

impl OracleProviderConfig {
    pub fn default_for(asset: Address, decimals: u32) -> Self {
        Self {
            base_asset: asset,
            oracle_type: OracleType::None,
            exchange_source: ExchangeSource::SpotOnly,
            asset_decimals: decimals,
            tolerance: OraclePriceFluctuation {
                first_upper_ratio_bps: 0,
                first_lower_ratio_bps: 0,
                last_upper_ratio_bps: 0,
                last_lower_ratio_bps: 0,
            },
            max_price_stale_seconds: 0,
        }
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketOracleConfigInput {
    pub exchange_source: ExchangeSource,
    pub max_price_stale_seconds: u64,
    pub first_tolerance_bps: i128,
    pub last_tolerance_bps: i128,
    pub cex_oracle: Address,
    pub cex_asset_kind: ReflectorAssetKind,
    pub cex_symbol: Symbol,
    pub dex_oracle: Option<Address>,
    pub dex_asset_kind: ReflectorAssetKind,
    /// DEX-side symbol passed to the DEX Reflector feed. The controller
    /// probes `dex_client.lastprice(...)` at configuration time and rejects
    /// unresolvable symbols with `OracleError::InvalidTicker`.
    pub dex_symbol: Symbol,
    pub twap_records: u32,
}

// ---------------------------------------------------------------------------
// Price feeds
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Market index
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketIndex {
    pub borrow_index_ray: i128,
    pub supply_index_ray: i128,
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
    pub actual_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolStrategyMutation {
    pub position: AccountPosition,
    pub market_index: MarketIndex,
    pub actual_amount: i128,
    pub amount_received: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolSyncData {
    pub params: MarketParams,
    pub state: PoolState,
}

// ---------------------------------------------------------------------------
// Position limits
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug)]
pub struct PositionLimits {
    pub max_borrow_positions: u32,
    pub max_supply_positions: u32,
}

// ---------------------------------------------------------------------------
// Liquidation
// ---------------------------------------------------------------------------

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

/// Named entry for a seized collateral asset produced by `execute_liquidation`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SeizeEntry {
    pub asset: Address,
    pub amount: i128,
    pub protocol_fee: i128,
    pub feed: PriceFeed,
    pub market_index: MarketIndex,
}

/// Named entry for a repaid debt asset produced by `execute_liquidation`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct RepayEntry {
    pub asset: Address,
    pub amount: i128,
    pub usd_wad: i128,
    pub feed: PriceFeed,
    pub market_index: MarketIndex,
}

/// Aggregate result of `execute_liquidation`.
#[derive(Clone)]
pub struct LiquidationResult {
    pub seized: Vec<SeizeEntry>,
    pub repaid: Vec<RepayEntry>,
    pub refunds: Vec<Payment>,
    pub max_debt_usd: i128,
    pub bonus_bps: i128,
}

// ---------------------------------------------------------------------------
// Aggregator Swap types
//
// These mirror EXACTLY the public ABI of `stellar-router-contract`
// (XOXNO's deployed Stellar aggregator). The off-chain quote builder
// produces an `AggregatorSwap` value that the controller forwards to the
// router via `batch_execute`. The router's own `BatchSwap` struct
// additionally carries `sender`, which the controller fills in with its
// own contract address — the user never sets it, eliminating spoofing.
//
// **DO NOT rename fields or reorder enum variants** without updating
// `stellar-router-contract/src/types.rs` AND
// `stellar-indexer/src/transaction/abi.rs` in lockstep. Soroban's
// `#[contracttype]` derives an alphabetical `ScMap` key encoding, so
// the bytes have to match across all three crates.
// ---------------------------------------------------------------------------

/// Which DEX/venue routes a given hop. Tag-only enum — every hop is
/// dispatched on this discriminant inside the router contract.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapVenue {
    Soroswap,
    Aquarius,
    Phoenix,
    NativeAmm,
    StaticBridge,
}

/// Single hop in a path. `pool`, `token_in`, `token_out` are all Soroban
/// `Address` values; Classic assets are pre-resolved to their SAC
/// contract IDs by the off-chain builder.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapHop {
    /// Fee in basis points (1 bps = 0.01%). Informational; the pool has
    /// authority over actual fees applied.
    pub fee_bps: u32,
    /// Pool contract address (for Soroswap/Aquarius/Phoenix), LP account
    /// (for NativeAmm), or zero bytes (for StaticBridge).
    pub pool: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub venue: SwapVenue,
}

/// One path in a (possibly multi-path) swap.
///
/// `split_ppm` is parts per million of the total input allocated to this
/// path. The router computes per-path input as
/// `total_in * split_ppm / 1_000_000`; the LAST path absorbs PPM rounding
/// so the entire `total_in` is consumed and no dust is left on the
/// sender. Within a path, output of hop N feeds hop N+1 directly — there
/// are no per-hop or per-path amount fields. The single `total_min_out`
/// guard at the router-batch level is the only slippage gate.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapPath {
    pub hops: Vec<SwapHop>,
    /// Parts per million of the total input. `> 0`; sum across all paths
    /// must equal `1_000_000`.
    pub split_ppm: u32,
}

/// User-facing aggregator swap request. The controller wraps this in
/// `BatchSwap` (filling `sender = current_contract_address` and
/// `total_in = actual_withdrawn`) before dispatching to the router.
/// Off-chain callers produce this directly from the indexer's quote
/// response.
#[contracttype]
#[derive(Clone, Debug)]
pub struct AggregatorSwap {
    /// One or more paths that all converge on the same final
    /// `token_out`. Each path's `split_ppm` declares its share of the
    /// total input.
    pub paths: Vec<SwapPath>,
    /// Aggregate slippage floor across all paths. Computed off-chain
    /// against quoted `amount_out * (1 - slippage)`. Must be > 0.
    pub total_min_out: i128,
}

/// Full batch passed to `Router::batch_execute`. Internal — strategy
/// endpoints take [`AggregatorSwap`] and the controller fills `sender`,
/// `total_in`, and `referral_id = 0` (lending strategies never charge
/// fees on user collateral / debt operations; the standalone swap UI
/// is the path that uses non-zero `referral_id`).
#[contracttype]
#[derive(Clone, Debug)]
pub struct BatchSwap {
    pub paths: Vec<SwapPath>,
    /// Referral ID for fee attribution. `0` means no fee. Lending
    /// strategies always pass `0`; only direct user→router swaps via
    /// the standalone swap UI use a non-zero referral ID.
    pub referral_id: u64,
    pub sender: Address,
    pub total_in: i128,
    pub total_min_out: i128,
}

// ---------------------------------------------------------------------------
// Consolidated storage types
// ---------------------------------------------------------------------------

/// Market lifecycle status.
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
    pub oracle_config: OracleProviderConfig,
    pub cex_oracle: Option<Address>,
    pub cex_asset_kind: ReflectorAssetKind,
    pub cex_symbol: Symbol,
    pub cex_decimals: u32,
    pub dex_oracle: Option<Address>,
    pub dex_asset_kind: ReflectorAssetKind,
    /// DEX-side symbol passed to the DEX Reflector feed. See
    /// `MarketOracleConfigInput::dex_symbol`.
    pub dex_symbol: Symbol,
    pub dex_decimals: u32,
    pub twap_records: u32,
}

/// Per-account state read and written by user operations.
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

// ---------------------------------------------------------------------------
// Storage key enums
// ---------------------------------------------------------------------------

/// Controller contract storage keys. Small integer fields use u32 because
/// `#[contracttype]` enum variant data does not support u8.
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
    EModeAssets(u32),
    AssetEModes(Address),
    IsolatedDebt(Address),
    PoolsList(u32),
    PoolsCount,
}

/// Pool storage keys, all Instance-scoped.
#[contracttype]
#[derive(Clone, Debug)]
pub enum PoolKey {
    Params,
    State,
}

/// Mutable pool state held in a single Instance entry.
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
