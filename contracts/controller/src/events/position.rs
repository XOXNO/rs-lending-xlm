//! Position-domain events: batch updates, liquidation, and flash loans.

use soroban_sdk::{contractevent, Address, Vec};

use super::{EventAccountAttributes, EventBorrowDelta, EventDepositDelta};

#[contractevent(topics = ["position", "batch_update"], data_format = "vec")]
#[derive(Clone, Debug)]
pub struct UpdatePositionBatchEvent {
    /// Account whose positions changed.
    pub account_id: u64,
    pub account_attributes: EventAccountAttributes,
    /// Collateral-side deltas recorded during the successful transaction.
    pub deposits: Vec<EventDepositDelta>,
    /// Debt-side deltas recorded during the successful transaction.
    pub borrows: Vec<EventBorrowDelta>,
}

/// Attributes a liquidation to its caller and carries the aggregate USD repaid
/// and the applied bonus rate. Per-asset repaid/seized token amounts ride the
/// position batch legs; total seized USD is
/// `repaid_usd_wad * (1 + bonus_bps / 10_000)`.
#[contractevent(topics = ["position", "liquidation"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiquidationEvent {
    pub liquidator: Address,
    pub account_id: u64,
    pub repaid_usd_wad: i128,
    pub bonus_bps: i128,
}

#[contractevent(topics = ["position", "flash_loan"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlashLoanEvent {
    pub hub_id: u32,
    pub asset: Address,
    pub receiver: Address,
    pub caller: Address,
    pub amount: i128,
    pub fee: i128,
}
