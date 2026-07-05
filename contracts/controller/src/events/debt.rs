//! Debt-lifecycle events (bad-debt cleanup).

use soroban_sdk::contractevent;

#[contractevent(topics = ["debt", "bad_debt"])]
#[derive(Clone, Debug)]
pub struct CleanBadDebtEvent {
    pub account_id: u64,
    /// Debt written off by cleanup, in USD WAD.
    pub total_borrow_usd_wad: i128,
    /// Collateral seized by cleanup, in USD WAD.
    pub total_collateral_usd_wad: i128,
}
