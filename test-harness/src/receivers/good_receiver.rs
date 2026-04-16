use soroban_sdk::{contract, contractimpl, token, Address, Bytes, Env};

/// A mock flash loan receiver that correctly repays the borrowed amount + fee.
#[contract]
pub struct GoodFlashLoanReceiver;

#[contractimpl]
impl GoodFlashLoanReceiver {
    /// Called by the controller during flash loan execution.
    /// The pool sent `amount` tokens to this contract via `flash_loan_begin`.
    /// The pool will pull `amount + fee` via `flash_loan_end`.
    /// Mints the `fee` portion so repayment succeeds.
    pub fn execute_flash_loan(
        env: Env,
        _initiator: Address,
        asset: Address,
        _amount: i128,
        fee: i128,
        _data: Bytes,
    ) {
        let tok_admin = token::StellarAssetClient::new(&env, &asset);
        tok_admin.mint(&env.current_contract_address(), &fee);
    }
}
