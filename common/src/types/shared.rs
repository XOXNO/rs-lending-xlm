//! Shared position-kind and account-mode enums; `Payment` for Certora harness.

use soroban_sdk::{contracttype, Address};

/// Asset-native amount keyed by token address (Certora harness; multi-hub uses hub keys).
pub type Payment = (Address, i128);

/// Side of an account position (supply vs borrow).
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AccountPositionType {
    Deposit = 1,
    Borrow = 2,
}

/// Account strategy/position mode discriminant.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PositionMode {
    Normal = 0,
    Multiply = 1,
    Long = 2,
    Short = 3,
}
