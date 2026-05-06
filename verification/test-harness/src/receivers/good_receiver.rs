use common::errors::GenericError;
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    contract, contractimpl, panic_with_error, token, Address, Bytes, Env, IntoVal, Symbol, Vec,
};

/// A mock flash loan receiver that correctly repays the borrowed amount + fee.
#[contract]
pub struct GoodFlashLoanReceiver;

#[contractimpl]
impl GoodFlashLoanReceiver {
    /// Called by the controller during flash loan execution.
    /// The pool sent `amount` tokens to this contract.
    /// The pool will pull `amount + fee` after this callback.
    /// Mints the `fee` portion so repayment succeeds.
    pub fn execute_flash_loan(
        env: Env,
        _initiator: Address,
        asset: Address,
        amount: i128,
        fee: i128,
        pool: Address,
        _data: Bytes,
    ) {
        let tok_admin = token::StellarAssetClient::new(&env, &asset);
        tok_admin.mint(&env.current_contract_address(), &fee);

        let total = amount
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
        let expiration_ledger = env
            .ledger()
            .sequence()
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));

        authorize_token_approve(&env, &asset, &pool, total, expiration_ledger);
        token::Client::new(&env, &asset).approve(
            &env.current_contract_address(),
            &pool,
            &total,
            &expiration_ledger,
        );
    }
}

fn authorize_token_approve(
    env: &Env,
    asset: &Address,
    spender: &Address,
    amount: i128,
    expiration_ledger: u32,
) {
    let token_approve = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: asset.clone(),
            fn_name: Symbol::new(env, "approve"),
            args: (
                env.current_contract_address(),
                spender.clone(),
                amount,
                expiration_ledger,
            )
                .into_val(env),
        },
        sub_invocations: Vec::new(env),
    });
    let mut auth_entries: Vec<InvokerContractAuthEntry> = Vec::new(env);
    auth_entries.push_back(token_approve);
    env.authorize_as_current_contract(auth_entries);
}
