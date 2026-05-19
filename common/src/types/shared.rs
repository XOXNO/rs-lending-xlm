use soroban_sdk::{contracttype, Address};

// Asset + amount pair.
pub type Payment = (Address, i128);

// Position discriminants.
pub const POSITION_TYPE_DEPOSIT: u32 = 1;
pub const POSITION_TYPE_BORROW: u32 = 2;

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
