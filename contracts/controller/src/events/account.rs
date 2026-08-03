use soroban_sdk::{contractevent, Address};

#[contractevent(topics = ["account", "delegate"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountDelegateEvent {
    pub account_id: u64,
    pub owner: Address,
    pub delegate: Address,
    pub granted: bool,
}
