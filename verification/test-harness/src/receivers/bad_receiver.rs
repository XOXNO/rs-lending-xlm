use soroban_sdk::{contract, contractimpl, Address, Bytes, Env};

/// A mock flash loan receiver that does NOT repay -- for testing rejection.
#[contract]
pub struct BadFlashLoanReceiver;

#[contractimpl]
impl BadFlashLoanReceiver {
    /// Called by the controller during flash loan execution.
    /// This receiver does nothing -- the pool will fail to pull repayment.
    pub fn execute_flash_loan(
        _env: Env,
        _initiator: Address,
        _asset: Address,
        _amount: i128,
        _fee: i128,
        _data: Bytes,
    ) {
        // Intentionally does nothing -- repayment will fail.
    }
}
