use soroban_sdk::{contractevent, Address};

/// Emitted when an account owner grants or revokes delegate authority.
///
/// Only published when the delegate set actually changed — re-granting an
/// existing delegate or revoking an absent one is a no-op and emits nothing, so
/// an indexer can reconstruct the delegate set by replaying these events.
#[contractevent(topics = ["account", "delegate"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountDelegateEvent {
    pub account_id: u64,
    pub owner: Address,
    pub delegate: Address,
    pub granted: bool,
}
