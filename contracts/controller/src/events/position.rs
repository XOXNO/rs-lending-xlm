//! Defines the contract events emitted for position-level changes: batched
//! supply/borrow deltas, liquidations, and flash loans.

use soroban_sdk::{contractevent, Address, Vec};

use super::{EventAccountAttributes, EventBorrowDelta, EventDepositDelta};

/// A batch of supply and borrow position changes for one account.
///
/// One operation may publish MORE THAN ONE batch: a `SeizeMode::Credit`
/// liquidation publishes the liquidated account's first, the receiver's
/// second. Key on `account_id`, never on one-batch-per-operation.
#[contractevent(topics = ["position", "batch_update"], data_format = "vec")]
#[derive(Clone, Debug)]
pub struct UpdatePositionBatchEvent {
    pub account_id: u64,
    pub account_attributes: EventAccountAttributes,

    pub deposits: Vec<EventDepositDelta>,

    pub borrows: Vec<EventBorrowDelta>,
}

/// `liquidator` liquidates `account_id` at a `bonus_bps` bonus.
///
/// `repaid_usd_wad` is what the pool actually received — net of refunded
/// overpayment and of any under-delivery — so it matches the debt retired and
/// the accompanying `LiqRepay` legs.
///
/// Carries NO seizure or fee figure: those are the batch's `LiqSeize` legs
/// (gross) and, in share-credit mode, its `LiqCredit` legs (net).
#[contractevent(topics = ["position", "liquidation"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiquidationEvent {
    pub liquidator: Address,
    pub account_id: u64,
    pub repaid_usd_wad: i128,
    pub bonus_bps: i128,
}

/// `caller` flash-loans `amount` of `asset` in `hub_id` to `receiver`.
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

/// `caller` opened a zero-fee flash position on `account_id`, forwarding
/// `amount_received` to `receiver`. Unlike a flash loan this mints debt.
#[contractevent(topics = ["position", "flash_position"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlashPositionEvent {
    pub account_id: u64,
    pub hub_id: u32,
    pub asset: Address,
    pub receiver: Address,
    pub caller: Address,
    pub amount: i128,
    pub amount_received: i128,
    pub fee: i128,
}
