//! Shared position-kind and account-mode enums; payment-leg aliases.

use soroban_sdk::{contracttype, Address, Vec};

use crate::types::pool::HubAssetKey;

/// Asset-native amount keyed by token address (Certora harness; multi-hub uses hub keys).
pub type Payment = (Address, i128);

/// Hub asset plus amount (one payment leg).
pub type HubPayment = (HubAssetKey, i128);

/// Deduped payment rows after aggregation.
pub type AggregatedPayments = Vec<HubPayment>;

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
