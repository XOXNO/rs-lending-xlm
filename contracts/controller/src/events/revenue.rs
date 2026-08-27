//! Defines the contract event emitted when accrued protocol revenue is claimed
//! from the pool and forwarded to the accumulator.

use soroban_sdk::{contractevent, Address};

/// Event recording that `caller` claimed `amount` of accrued protocol revenue
/// for `(hub_id, asset)` and forwarded it to `accumulator`.
///
/// `amount` is the MEASURED balance delta the controller received from the
/// pool and then forwarded — not the pool's reported figure. A token that
/// delivers less than it is sent moves the smaller number, and this event
/// carries that number (F-8, INV-ACCT-03). Only published when it is positive,
/// so a keeper sweeping a long asset list does not write a row per empty
/// market.
///
/// This is the complement to the `revenue` field on the pool's
/// `PoolMarketStateEvent`, which is OUTSTANDING unclaimed revenue and is
/// decremented by every claim. Neither is monotonic alone; lifetime protocol
/// revenue for a market is
///
/// ```text
/// outstanding (revenue shares valued at the supply index) + Σ ClaimRevenueEvent.amount
/// ```
///
/// Claims may be partial: `burn_claimable_revenue` caps the burn at the
/// market's available cash, so one market can emit many of these over time.
#[contractevent(topics = ["revenue", "claim"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimRevenueEvent {
    pub hub_id: u32,
    pub asset: Address,
    pub caller: Address,
    pub accumulator: Address,
    pub amount: i128,
}
