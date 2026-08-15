use soroban_sdk::{contractevent, Address};

/// Records the initial payment asset and amount supplied when opening a multiply
/// position, before it is converted into the position's collateral asset.
#[contractevent(topics = ["strategy", "initial_payment"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialMultiplyPaymentEvent {
    pub token: Address,
    pub amount: i128,
    pub account_id: u64,
}

/// Records the result of migrating an account's position from an external Blend
/// pool into the hub, including the number of collateral, supply, and debt
/// positions moved.
#[contractevent(topics = ["strategy", "blend_migration"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlendMigrationEvent {
    pub account_id: u64,
    pub blend_pool: Address,
    pub collateral_count: u32,
    pub supply_count: u32,
    pub debt_count: u32,
}
