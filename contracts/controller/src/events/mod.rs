//! Controller event schema and shared wire encodings.
//!
//! Vec-encoded delta field order is stable ABI; off-chain decoders depend on
//! the enum discriminants. Domain event types live in sibling modules and are
//! re-exported here so callers keep `crate::events::…` paths.

use soroban_sdk::{contracttype, Address};

use common::types::{Account, AccountMeta, AccountPosition, DebtPosition, PositionMode};

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
#[derive(Clone, Debug, Eq, PartialEq)]
/// Account attributes; field order is wire ABI.
pub struct EventAccountAttributes(pub Address, pub u32, pub EventPositionMode);

impl From<&Account> for EventAccountAttributes {
    fn from(value: &Account) -> Self {
        Self(value.owner.clone(), value.spoke_id, value.mode.into())
    }
}

impl From<&AccountMeta> for EventAccountAttributes {
    fn from(value: &AccountMeta) -> Self {
        Self(value.owner.clone(), value.spoke_id, value.mode.into())
    }
}

/// Position action stored as a stable `u32` discriminant.
/// Off-chain decoders depend on these values.
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

/// Collateral-side position delta, vec-encoded for client compatibility.
/// Field order is wire ABI; do not reorder:
/// `[action, hub_id, asset, scaled_amount, index_ray, amount,
///   liquidation_threshold, liquidation_bonus, loan_to_value,
///   liquidation_fees]`.
/// Risk params are the position entry values.
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

/// Debt-side position delta; field order is wire ABI.
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

mod config;
mod debt;
mod market;
mod position;
mod strategy;

pub use config::*;
pub use debt::*;
pub use market::*;
pub use position::*;
pub use strategy::*;

#[cfg(test)]
#[path = "../../tests/events.rs"]
mod tests;
