use soroban_sdk::{contract, contractimpl, Address, Bytes, Env};

/// Mock flash-loan receiver that does not repay.
#[contract]
pub struct BadFlashLoanReceiver;

#[contractimpl]
impl BadFlashLoanReceiver {
    /// Flash-loan callback that leaves repayment unpaid.
    pub fn execute_flash_loan(
        _env: Env,
        _initiator: Address,
        _asset: Address,
        _amount: i128,
        _fee: i128,
        _pool: Address,
        _data: Bytes,
    ) {
        // Intentionally does nothing -- repayment will fail.
    }
}
