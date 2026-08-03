use soroban_sdk::{contracttype, Address, Vec};

use crate::types::pool::HubAssetKey;

pub type Payment = (Address, i128);

pub type HubPayment = (HubAssetKey, i128);

pub type AggregatedPayments = Vec<HubPayment>;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AccountPositionType {
    Deposit = 1,
    Borrow = 2,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PositionMode {
    Normal = 0,
    Multiply = 1,
    Long = 2,
    Short = 3,
}
