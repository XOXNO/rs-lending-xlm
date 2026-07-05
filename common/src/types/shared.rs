//! Cross-domain shared types: the legacy `Payment` alias and the position-kind
//! ([`AccountPositionType`]) and account-mode ([`PositionMode`]) enums used by
//! both the pool and controller.

use soroban_sdk::{contracttype, Address};

/// Asset-native amount keyed by token address.
///
/// Legacy tuple shape referenced only by the certora specification harness;
/// no current controller entrypoint uses this alias (multi-hub payments use
/// `(HubAssetKey, i128)` instead).
pub type Payment = (Address, i128);

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AccountPositionType {
    /// Collateral supply position.
    Deposit = 1,
    /// Borrow debt position.
    Borrow = 2,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PositionMode {
    /// Plain lending account with no strategy mode.
    Normal = 0,
    /// Leveraged collateral/debt loop.
    Multiply = 1,
    /// Long exposure strategy.
    Long = 2,
    /// Short exposure strategy.
    Short = 3,
}
