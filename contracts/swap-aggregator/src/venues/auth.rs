//! Invoker-contract auth for token transfers and nested pool calls.

use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    vec, Address, Env, IntoVal, Symbol, Val,
};

/// Authorize `token.transfer(from, to, amount)` as the current contract.
pub(crate) fn authorize_token_transfer(
    env: &Env,
    token: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
) {
    authorize_as_current(
        env,
        token,
        "transfer",
        vec![
            env,
            from.into_val(env),
            to.into_val(env),
            amount.into_val(env),
        ],
    );
}

/// Authorize `token.approve(owner, spender, amount, expiration)`.
pub(crate) fn authorize_token_approve(
    env: &Env,
    token: &Address,
    owner: &Address,
    spender: &Address,
    amount: i128,
    expiration_ledger: u32,
) {
    authorize_as_current(
        env,
        token,
        "approve",
        vec![
            env,
            owner.into_val(env),
            spender.into_val(env),
            amount.into_val(env),
            expiration_ledger.into_val(env),
        ],
    );
}

/// Build one invoker-auth entry for `contract.fn_name(args)`.
pub(crate) fn auth_entry(
    env: &Env,
    contract: &Address,
    fn_name: &str,
    args: soroban_sdk::Vec<Val>,
    sub_invocations: soroban_sdk::Vec<InvokerContractAuthEntry>,
) -> InvokerContractAuthEntry {
    InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: contract.clone(),
            fn_name: Symbol::new(env, fn_name),
            args,
        },
        sub_invocations,
    })
}

/// Register a single top-level invoker auth for the current contract call.
pub(crate) fn authorize_as_current(
    env: &Env,
    contract: &Address,
    fn_name: &str,
    args: soroban_sdk::Vec<Val>,
) {
    env.authorize_as_current_contract(vec![
        env,
        auth_entry(env, contract, fn_name, args, vec![env]),
    ]);
}
