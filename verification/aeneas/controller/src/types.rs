//! Stand-in for the slice of `common::types` that the included controller
//! files use. Every struct below has exactly the fields of its production
//! counterpart (`common/src/types/{controller,oracle,pool,shared}.rs`), and the
//! `From` conversions are copied verbatim. The production types are
//! `#[contracttype]` (an XDR serialisation derive), which changes nothing about
//! their in-memory fields. `Account::mode` is omitted (no included function
//! reads it).

use crate::math::fp::{Bps, Ray, Wad};
use soroban_sdk::{Address, Env, Map, Vec};

/// `common/src/types/controller.rs`.
#[derive(Clone, Debug)]
pub struct SpokeConfig {
    pub is_deprecated: bool,
    pub liquidation_target_hf_wad: i128,
    pub hf_for_max_bonus_wad: i128,
    pub liquidation_bonus_factor_bps: u32,
}

/// `common/src/types/pool.rs`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HubAssetKey {
    pub hub_id: u32,
    pub asset: Address,
}

/// `common/src/types/shared.rs`.
pub type HubPayment = (HubAssetKey, i128);

/// `common/src/types/shared.rs`.
pub type AggregatedPayments = Vec<HubPayment>;

/// `common/src/types/shared.rs`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AccountPositionType {
    Deposit = 1,
    Borrow = 2,
}

/// `common/src/types/oracle.rs`.
#[derive(Clone, Debug)]
pub struct PriceFeedRaw {
    pub price_wad: i128,
    pub asset_decimals: u32,
    pub timestamp: u64,
}

/// `common/src/types/oracle.rs`.
#[derive(Clone, Copy, Debug)]
pub struct PriceFeed {
    pub price: Wad,
    pub asset_decimals: u32,
    pub timestamp: u64,
}

impl PriceFeed {
    /// Converts a raw token amount (scaled by `asset_decimals`) to its WAD-scaled USD value
    /// at this feed's price, rounding half up.
    pub fn usd_value_wad(self, env: &Env, token_amount: i128) -> Wad {
        Wad::from_token(env, token_amount, self.asset_decimals).mul(env, self.price)
    }
}

impl From<&PriceFeedRaw> for PriceFeed {
    fn from(r: &PriceFeedRaw) -> Self {
        Self {
            price: Wad::from(r.price_wad),
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

/// `common/src/types/pool.rs`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketIndexRaw {
    pub borrow_index: i128,
    pub supply_index: i128,
}

/// `common/src/types/pool.rs`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarketIndex {
    pub borrow_index: Ray,
    pub supply_index: Ray,
}

impl From<&MarketIndexRaw> for MarketIndex {
    fn from(r: &MarketIndexRaw) -> Self {
        Self {
            borrow_index: Ray::from(r.borrow_index),
            supply_index: Ray::from(r.supply_index),
        }
    }
}

impl From<&MarketIndex> for MarketIndexRaw {
    fn from(t: &MarketIndex) -> Self {
        Self {
            borrow_index: t.borrow_index.raw(),
            supply_index: t.supply_index.raw(),
        }
    }
}

/// `common/src/types/pool.rs`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPositionRaw {
    pub scaled_amount: i128,
    pub liquidation_threshold: u32,
    pub liquidation_bonus: u32,
    pub loan_to_value: u32,
    pub liquidation_fees: u32,
}

/// `common/src/types/pool.rs`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountPosition {
    pub scaled_amount: Ray,
    pub liquidation_threshold: Bps,
    pub liquidation_bonus: Bps,
    pub loan_to_value: Bps,
    pub liquidation_fees: Bps,
}

impl From<&AccountPositionRaw> for AccountPosition {
    fn from(r: &AccountPositionRaw) -> Self {
        Self {
            scaled_amount: Ray::from(r.scaled_amount),
            liquidation_threshold: Bps::from(i128::from(r.liquidation_threshold)),
            liquidation_bonus: Bps::from(i128::from(r.liquidation_bonus)),
            loan_to_value: Bps::from(i128::from(r.loan_to_value)),
            liquidation_fees: Bps::from(i128::from(r.liquidation_fees)),
        }
    }
}

impl From<&AccountPosition> for AccountPositionRaw {
    fn from(t: &AccountPosition) -> Self {
        Self {
            scaled_amount: t.scaled_amount.raw(),
            liquidation_threshold: t.liquidation_threshold.raw() as u32,
            liquidation_bonus: t.liquidation_bonus.raw() as u32,
            loan_to_value: t.loan_to_value.raw() as u32,
            liquidation_fees: t.liquidation_fees.raw() as u32,
        }
    }
}

/// `common/src/types/pool.rs`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScaledPositionRaw {
    pub scaled_amount: i128,
}

impl From<&AccountPosition> for ScaledPositionRaw {
    fn from(t: &AccountPosition) -> Self {
        Self {
            scaled_amount: t.scaled_amount.raw(),
        }
    }
}

impl From<&DebtPosition> for ScaledPositionRaw {
    fn from(t: &DebtPosition) -> Self {
        Self {
            scaled_amount: t.scaled_amount.raw(),
        }
    }
}

/// `common/src/types/pool.rs`.
#[derive(Clone, Debug)]
pub struct PoolAction {
    pub position: ScaledPositionRaw,

    pub amount: i128,
    pub hub_asset: HubAssetKey,
}

/// `common/src/types/pool.rs`.
#[derive(Clone, Debug)]
pub struct PoolWithdrawEntry {
    pub action: PoolAction,

    pub protocol_fee: i128,
}

/// `common/src/types/pool.rs`.
#[derive(Clone, Debug)]
pub struct PoolSeizeEntry {
    pub hub_asset: HubAssetKey,
    pub side: AccountPositionType,
    pub position: ScaledPositionRaw,
}

/// `common/src/types/pool.rs` (the pool's per-leg mutation result). The
/// included code never reads it; its fields are left out.
#[derive(Clone, Debug)]
pub struct PoolPositionMutation {
    _opaque: (),
}

/// `common/src/types/controller.rs`.
#[derive(Clone, Debug)]
pub struct AssetConfig {
    pub loan_to_value: Bps,

    pub liquidation_threshold: Bps,

    pub liquidation_bonus: Bps,

    pub liquidation_fees: Bps,
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
}

/// `common/src/types/pool.rs`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebtPositionRaw {
    pub scaled_amount: i128,
}

/// `common/src/types/pool.rs`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DebtPosition {
    pub scaled_amount: Ray,
}

impl From<&DebtPositionRaw> for DebtPosition {
    fn from(r: &DebtPositionRaw) -> Self {
        Self {
            scaled_amount: Ray::from(r.scaled_amount),
        }
    }
}

/// `common/src/types/controller.rs`.
#[derive(Clone, Debug)]
pub struct PaymentTuple {
    pub asset: Address,
    pub amount: i128,
}

/// `common/src/types/controller.rs`.
#[derive(Clone, Debug)]
pub struct SeizeEntry {
    pub hub_asset: HubAssetKey,
    pub amount: i128,
    pub protocol_fee: i128,
    pub scaled_amount: i128,
    pub bonus_scaled: i128,
    pub liquidation_fees: u32,
    pub feed: PriceFeedRaw,
    pub market_index: MarketIndexRaw,
}

/// `common/src/types/controller.rs`.
#[derive(Clone, Debug)]
pub struct RepayEntry {
    pub hub_asset: HubAssetKey,
    pub amount: i128,
    pub usd_wad: i128,
    pub feed: PriceFeedRaw,
    pub market_index: MarketIndexRaw,
}

/// `common/src/types/controller.rs`.
#[derive(Clone, Debug)]
pub struct LiquidationResult {
    pub seized: Vec<SeizeEntry>,
    pub repaid: Vec<RepayEntry>,
    pub refunds: Vec<PaymentTuple>,
    pub max_debt_usd: i128,
    pub bonus_bps: i128,
}

/// `common/src/types/controller.rs` (without `mode`).
#[derive(Clone, Debug)]
pub struct Account {
    pub owner: Address,
    pub spoke_id: u32,
    pub supply_positions: Map<HubAssetKey, AccountPositionRaw>,
    pub borrow_positions: Map<HubAssetKey, DebtPositionRaw>,
}

impl Account {
    /// Copied verbatim from `common/src/types/controller.rs`.
    pub fn get_or_create_supply_position(
        &self,
        hub_asset: &HubAssetKey,
        config: &AssetConfig,
    ) -> AccountPosition {
        self.supply_positions
            .get(hub_asset.clone())
            .map(|raw| AccountPosition::from(&raw))
            .unwrap_or(AccountPosition {
                scaled_amount: Ray::ZERO,
                liquidation_threshold: config.liquidation_threshold,
                liquidation_bonus: config.liquidation_bonus,
                loan_to_value: config.loan_to_value,
                liquidation_fees: config.liquidation_fees,
            })
    }
}
