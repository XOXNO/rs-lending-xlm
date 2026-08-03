use soroban_sdk::{contractevent, Address};

#[contractevent(topics = ["strategy", "initial_payment"])]
#[derive(Clone, Debug, Eq, PartialEq)]

pub struct InitialMultiplyPaymentEvent {
    pub token: Address,
    pub amount: i128,
    pub account_id: u64,
}

#[contractevent(topics = ["strategy", "blend_migration"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlendMigrationEvent {
    pub account_id: u64,
    pub blend_pool: Address,
    pub collateral_count: u32,
    pub supply_count: u32,
    pub debt_count: u32,
}
