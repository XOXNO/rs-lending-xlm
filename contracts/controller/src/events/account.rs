use soroban_sdk::{contractevent, Address};

/// Event recording a change in delegate authorization for an account: either
/// granting or revoking `delegate`'s ability to act on behalf of `owner` for
/// `account_id`, depending on the `granted` flag.
#[contractevent(topics = ["account", "delegate"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountDelegateEvent {
    pub account_id: u64,
    pub owner: Address,
    pub delegate: Address,
    pub granted: bool,
}
