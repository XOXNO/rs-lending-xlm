use soroban_sdk::{contracttype, Address};

/// Asset-native amount keyed by token address.
///
/// Kept as a tuple for existing ABI compatibility. Prefer `PaymentTuple` for
/// new event or view payloads that need named fields.
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
