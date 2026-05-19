use crate::types::oracle::MarketOracleConfig;
use crate::types::pool::AccountPosition;
use crate::types::shared::PositionMode;
use soroban_sdk::{contracttype, Address, Map, Vec};

#[contracttype]
#[derive(Clone, Debug)]
pub struct AssetConfig {
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
    pub isolation_debt_ceiling_usd_wad: i128,
    pub flashloan_fee_bps: u32,
    pub borrow_cap: i128,
    pub supply_cap: i128,
    pub min_collat_floor_usd_wad: i128,
    pub min_debt_floor_usd_wad: i128,
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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountMeta {
    pub owner: Address,
    pub is_isolated: bool,
    pub e_mode_category_id: u32,
    pub mode: PositionMode,
    pub isolated_asset: Option<Address>,
}

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

#[contracttype]
#[derive(Clone, Debug)]
pub struct SeizeEntry {
    pub asset: Address,
    pub amount: i128,
    pub protocol_fee: i128,
    pub feed: crate::types::oracle::PriceFeed,
    pub market_index: crate::types::pool::MarketIndex,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RepayEntry {
    pub asset: Address,
    pub amount: i128,
    pub usd_wad: i128,
    pub feed: crate::types::oracle::PriceFeed,
    pub market_index: crate::types::pool::MarketIndex,
}

pub struct LiquidationResult {
    pub seized: Vec<SeizeEntry>,
    pub repaid: Vec<RepayEntry>,
    pub refunds: Vec<crate::types::shared::Payment>,
    pub max_debt_usd: i128,
    pub bonus_bps: i128,
}

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

#[contracttype]
#[derive(Clone, Debug)]
pub enum ControllerKey {
    PoolTemplate,
    Aggregator,
    Accumulator,
    AccountNonce,
    PositionLimits,
    LastEModeCategoryId,
    FlashLoanOngoing,
    Market(Address),
    AccountMeta(u64),
    SupplyPositions(u64),
    BorrowPositions(u64),
    EModeCategory(u32),
    IsolatedDebt(Address),
    PoolsList,
}
