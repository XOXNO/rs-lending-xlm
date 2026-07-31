use soroban_sdk::{contract, contractimpl, Address, Bytes, Env};

#[contract]
pub struct BadFlashLoanReceiver;

#[contractimpl]
impl BadFlashLoanReceiver {
    pub fn execute_flash_loan(
        _env: Env,
        _initiator: Address,
        _asset: Address,
        _amount: i128,
        _fee: i128,
        _pool: Address,
        _data: Bytes,
    ) {
    }
}
