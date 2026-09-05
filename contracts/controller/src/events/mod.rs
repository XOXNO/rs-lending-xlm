//! Controller events and shared position payloads.

use soroban_sdk::{contractevent, contracttype, Address, Vec};

use common::types::{Account, AccountPosition, DebtPosition, PositionMode};

/// Event encoding of [`PositionMode`]; only `Normal` is renamed to `None`.
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

/// Account identity tuple: `(owner, spoke_id, mode)`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]

pub struct EventAccountAttributes(pub Address, pub u32, pub EventPositionMode);

impl From<&Account> for EventAccountAttributes {
    fn from(value: &Account) -> Self {
        Self(value.owner.clone(), value.spoke_id, value.mode.into())
    }
}

/// Operation that produced a position change.
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
    RpColNet = 14,
    /// Collateral credited net of protocol fees; `LiqSeize` records gross seizure.
    LiqCredit = 15,
    /// Debt minted by `flash_position`.
    FlashPos = 16,
}

/// Supply-position delta: `(action, hub_id, asset, scaled_amount, index_ray,
/// amount, liquidation_threshold, liquidation_bonus, loan_to_value,
/// liquidation_fees)`. Scaled balance and index use RAY; amount uses asset
/// units. The last four fields are BPS risk parameters cast to `u32`.
///
/// `scaled_amount` is the resulting position balance; `amount` is this account's
/// movement. In credit mode, `LiqSeize` is gross and `LiqCredit` is net of fees.
/// Transfer mode withholds the fee from the payout, without a second credit leg.
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
            position.liquidation_fees.raw() as u32,
        )
    }
}

/// Borrow-position delta: `(action, hub_id, asset, scaled_amount, index_ray, amount)`.
/// Scaled balance and index use RAY; amount uses asset units. `scaled_amount`
/// is the resulting position balance.
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

/// Grants or revokes a delegate's authority over an account.
#[contractevent(topics = ["account", "delegate"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountDelegateEvent {
    pub account_id: u64,
    pub owner: Address,
    pub delegate: Address,
    pub granted: bool,
}

/// Bad-debt cleanup that seizes remaining positions and removes the account.
/// Values describe the account before cleanup, in USD (WAD).
#[contractevent(topics = ["debt", "bad_debt"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanBadDebtEvent {
    pub account_id: u64,

    pub total_borrow_usd_wad: i128,

    pub total_collateral_usd_wad: i128,
}

/// A batch of supply and borrow position changes for one account.
///
/// Credit-mode liquidation publishes the liquidated account's batch first,
/// then the receiver's, before any bad-debt cleanup. Index batches by
/// `account_id`; an operation can produce more than one.
#[contractevent(topics = ["position", "batch_update"], data_format = "vec")]
#[derive(Clone, Debug)]
pub struct UpdatePositionBatchEvent {
    pub account_id: u64,
    pub account_attributes: EventAccountAttributes,

    pub deposits: Vec<EventDepositDelta>,

    pub borrows: Vec<EventBorrowDelta>,
}

/// Liquidation repayment and bonus, in USD (WAD) and BPS respectively.
///
/// `repaid_usd_wad` measures debt retired after under-delivery and refunds,
/// matching the `LiqRepay` legs. Seizure and fees appear in position batches:
/// `LiqSeize` is gross; credit mode also emits net `LiqCredit` legs.
#[contractevent(topics = ["position", "liquidation"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiquidationEvent {
    pub liquidator: Address,
    pub account_id: u64,
    pub repaid_usd_wad: i128,
    pub bonus_bps: i128,
}

/// Completed flash loan; `amount` and `fee` use asset units.
#[contractevent(topics = ["position", "flash_loan"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlashLoanEvent {
    pub hub_id: u32,
    pub asset: Address,
    pub receiver: Address,
    pub caller: Address,
    pub amount: i128,
    pub fee: i128,
}

/// Debt-funded flash position with `fee = 0`. `amount` is requested debt;
/// `amount_received` is measured receiver delivery. Both use asset units.
#[contractevent(topics = ["position", "flash_position"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlashPositionEvent {
    pub account_id: u64,
    pub hub_id: u32,
    pub asset: Address,
    pub receiver: Address,
    pub caller: Address,
    pub amount: i128,
    pub amount_received: i128,
    pub fee: i128,
}

/// Protocol revenue claimed and forwarded to the accumulator.
///
/// `amount` is the positive controller-balance increase submitted for onward
/// transfer, in asset units; it does not measure the accumulator's receipt.
///
/// Claims reduce outstanding pool revenue and are capped by available cash.
/// With exact token delivery, accrued revenue is cumulative claims plus
/// outstanding pool revenue valued at the current supply index.
#[contractevent(topics = ["revenue", "claim"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimRevenueEvent {
    pub hub_id: u32,
    pub asset: Address,
    pub caller: Address,
    pub accumulator: Address,
    pub amount: i128,
}

/// Requested multiply payment in the original asset's units, before any
/// conversion. The amount does not measure the controller's receipt.
#[contractevent(topics = ["strategy", "initial_payment"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialMultiplyPaymentEvent {
    pub token: Address,
    pub amount: i128,
    pub account_id: u64,
}

/// Completed Blend migration, with collateral, supply and debt entry counts.
#[contractevent(topics = ["strategy", "blend_migration"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlendMigrationEvent {
    pub account_id: u64,
    pub blend_pool: Address,
    pub collateral_count: u32,
    pub supply_count: u32,
    pub debt_count: u32,
}

mod config;
mod market;

pub use config::*;
pub use market::*;

#[cfg(test)]
#[path = "../../tests/events.rs"]
mod tests;
