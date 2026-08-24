//! Shared scalar and tuple types used across the lending protocol crates:
//! payment tuples, the encoded swap-router payload, account position classification, and
//! position modes.

use soroban_sdk::{contracttype, Address, Bytes, Vec};

use crate::types::pool::HubAssetKey;

/// Encoded swap route passed to the aggregator router's `execute_strategy` entry point.
pub type StrategySwap = Bytes;

/// A token amount paired with the address of the underlying asset contract.
pub type Payment = (Address, i128);

/// A token amount paired with the `HubAssetKey` (hub identifier and asset
/// address) it is denominated in.
pub type HubPayment = (HubAssetKey, i128);

/// An ordered collection of `HubPayment` entries.
pub type AggregatedPayments = Vec<HubPayment>;

/// Classifies an account position as a deposit or a borrow.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AccountPositionType {
    Deposit = 1,
    Borrow = 2,
}

/// Identifies the trading mode of a position: plain lending, leveraged
/// multiply, or directional long/short exposure.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PositionMode {
    Normal = 0,
    Multiply = 1,
    Long = 2,
    Short = 3,
}
