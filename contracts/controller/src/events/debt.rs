//! Defines the contract event emitted when an account's remaining positions
//! are seized and the account is removed as part of bad-debt cleanup.

use soroban_sdk::contractevent;

/// Event recording that an account's outstanding debt and collateral
/// positions are seized and its entry removed because it holds bad debt.
#[contractevent(topics = ["debt", "bad_debt"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanBadDebtEvent {
    pub account_id: u64,

    pub total_borrow_usd_wad: i128,

    pub total_collateral_usd_wad: i128,
}
