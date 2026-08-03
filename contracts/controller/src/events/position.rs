use soroban_sdk::{contractevent, Address, Vec};

use super::{EventAccountAttributes, EventBorrowDelta, EventDepositDelta};

#[contractevent(topics = ["position", "batch_update"], data_format = "vec")]
#[derive(Clone, Debug)]
pub struct UpdatePositionBatchEvent {
    pub account_id: u64,
    pub account_attributes: EventAccountAttributes,

    pub deposits: Vec<EventDepositDelta>,

    pub borrows: Vec<EventBorrowDelta>,
}

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
