//! Contract-event types plus the shared building blocks the submodules use:
//! wire position mode, account attributes, the action tag, and the
//! deposit/borrow delta payloads.

use soroban_sdk::{contracttype, Address};

use common::types::{Account, AccountPosition, DebtPosition, PositionMode};

/// Wire form of [`PositionMode`]; `Normal` maps to `None`, the rest 1:1.
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

/// Tuple of `(owner, spoke_id, mode)` describing an account's identity and
/// position mode for inclusion in event payloads.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]

pub struct EventAccountAttributes(pub Address, pub u32, pub EventPositionMode);

impl From<&Account> for EventAccountAttributes {
    fn from(value: &Account) -> Self {
        Self(value.owner.clone(), value.spoke_id, value.mode.into())
    }
}

/// Tag identifying which controller operation produced a position-change
/// event.
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
    /// Collateral credited to a share-credit liquidator's receiving account.
    /// Separate from [`PositionAction::LiqSeize`] because the seizure leg is
    /// **gross** of the protocol fee and this one is **net**; one tag for both
    /// would overstate a liquidator's proceeds by the fee.
    LiqCredit = 15,
    /// Strategy-debt mint for `flash_position`.
    FlashPos = 16,
}

/// Supply-position delta: `(action, hub_id, asset, scaled_amount, index_ray,
/// amount, liquidation_threshold, liquidation_bonus, loan_to_value,
/// liquidation_fees)`; the last four are risk params truncated to `u32`.
///
/// `amount` is always this account's own movement, never a counterparty's.
/// In credit-mode liquidation the fee is `LiqSeize.amount - LiqCredit.amount`
/// (seize is gross, credit net); in transfer mode it is withheld from the
/// outbound transfer instead of appearing as a second leg.
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

mod account;
mod config;
mod debt;
mod market;
mod position;
mod revenue;
mod strategy;

pub use account::*;
pub use config::*;
pub use debt::*;
pub use market::*;
pub use position::*;
pub use revenue::*;
pub use strategy::*;

#[cfg(test)]
#[path = "../../tests/events.rs"]
mod tests;

/// Counterparty address plus action tag for emitting a position event.
pub(crate) struct EventContext {
    pub counterparty: soroban_sdk::Address,
    pub action: PositionAction,
}
