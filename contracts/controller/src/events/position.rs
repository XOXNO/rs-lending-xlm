//! Defines the contract events emitted for position-level changes: batched
//! supply/borrow deltas, liquidations, and flash loans.

use soroban_sdk::{contractevent, Address, Vec};

use super::{EventAccountAttributes, EventBorrowDelta, EventDepositDelta};

/// Event recording a batch of supply and borrow position changes for a
/// single account produced by one controller operation.
#[contractevent(topics = ["position", "batch_update"], data_format = "vec")]
#[derive(Clone, Debug)]
pub struct UpdatePositionBatchEvent {
    pub account_id: u64,
    pub account_attributes: EventAccountAttributes,

    pub deposits: Vec<EventDepositDelta>,

    pub borrows: Vec<EventBorrowDelta>,
}

/// Event recording that `liquidator` liquidates `account_id`, repaying
/// `repaid_usd_wad` of debt (USD, WAD scale) at a `bonus_bps` liquidation
/// bonus.
#[contractevent(topics = ["position", "liquidation"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiquidationEvent {
    pub liquidator: Address,
    pub account_id: u64,
    pub repaid_usd_wad: i128,
    pub bonus_bps: i128,
}

/// Event recording that `caller` executes a flash loan of `amount` of `asset`
/// in hub `hub_id` to `receiver`, charging `fee`.
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
