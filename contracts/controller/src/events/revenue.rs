//! Defines the contract event emitted when accrued protocol revenue is claimed
//! from the pool and forwarded to the accumulator.

use soroban_sdk::{contractevent, Address};

/// `caller` claimed `amount` of protocol revenue for `(hub_id, asset)` and
/// forwarded it to `accumulator`.
///
/// `amount` is the MEASURED delta the controller received and forwarded, not
/// the pool's reported figure (F-8, INV-ACCT-03). Published only when positive.
///
/// Complements the pool's `revenue`, which is OUTSTANDING unclaimed and is
/// decremented by every claim. Neither is monotonic alone: lifetime revenue is
/// `outstanding (valued at the supply index) + Σ these amounts`. Claims may be
/// partial — the burn is capped at the market's available cash.
#[contractevent(topics = ["revenue", "claim"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimRevenueEvent {
    pub hub_id: u32,
    pub asset: Address,
    pub caller: Address,
    pub accumulator: Address,
    pub amount: i128,
}
