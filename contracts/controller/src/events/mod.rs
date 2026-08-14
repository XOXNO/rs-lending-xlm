//! Re-exports the controller's contract-event types and shared event
//! building blocks: wire representations of position mode and account
//! attributes, the position-action tag, and the deposit/borrow delta payloads
//! used by the position-change events defined in the submodules.

use soroban_sdk::{contracttype, Address};

use common::types::{Account, AccountMeta, AccountPosition, DebtPosition, PositionMode};

/// Wire representation of [`PositionMode`] used in event payloads.
/// `PositionMode::Normal` maps to `None`; the other variants map 1:1.
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
    /// Maps `PositionMode::Normal` to `None` and the other variants 1:1.
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
    /// Builds an `EventAccountAttributes` from the account's owner, spoke id,
    /// and position mode.
    fn from(value: &Account) -> Self {
        Self(value.owner.clone(), value.spoke_id, value.mode.into())
    }
}

impl From<&AccountMeta> for EventAccountAttributes {
    /// Builds an `EventAccountAttributes` from the account meta's owner,
    /// spoke id, and position mode.
    fn from(value: &AccountMeta) -> Self {
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
}

/// Tuple of `(action, hub_id, asset, scaled_amount, index_ray, amount,
/// liquidation_threshold, liquidation_bonus, loan_to_value,
/// liquidation_fees)` describing a change to a supply position for inclusion
/// in event payloads. The last four fields are the position's risk
/// parameters, truncated to `u32`.
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
    /// Builds an `EventDepositDelta` from `action`, `hub_id`, `asset`,
    /// `index_ray`, `amount`, and the scaled amount and risk parameters read
    /// from `position`.
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

/// Tuple of `(action, hub_id, asset, scaled_amount, index_ray, amount)`
/// describing a change to a borrow position for inclusion in event payloads.
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
    /// Builds an `EventBorrowDelta` from `action`, `hub_id`, `asset`,
    /// `index_ray`, `amount`, and the scaled amount read from `position`.
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
mod strategy;

pub use account::*;
pub use config::*;
pub use debt::*;
pub use market::*;
pub use position::*;
pub use strategy::*;

#[cfg(test)]
#[path = "../../tests/events.rs"]
mod tests;

/// Bundles the counterparty address and action tag needed to emit a
/// position-change event for an operation.
pub(crate) struct EventContext {
    pub counterparty: soroban_sdk::Address,
    pub action: PositionAction,
}
