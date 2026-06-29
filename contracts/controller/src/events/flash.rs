use soroban_sdk::{contractevent, Address};

#[contractevent(topics = ["position", "flash_loan"])]
#[derive(Clone, Debug)]
pub struct FlashLoanEvent {
    pub asset: Address,
    pub receiver: Address,
    pub caller: Address,
    pub amount: i128,
    pub fee: i128,
}
