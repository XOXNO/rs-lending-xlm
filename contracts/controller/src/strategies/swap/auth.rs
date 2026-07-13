//! Router token-pull preauthorization.

use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{symbol_short, vec, Address, Env, IntoVal, Vec};

/// Authorizes exactly one `token_in.transfer(controller, router_addr, amount_in)`
/// sub-invocation. The router can pull no more than `amount_in` of `token_in`
/// and cannot redirect the transfer to any other recipient.
pub(crate) fn pre_authorize_router_pull(
    env: &Env,
    router_addr: &Address,
    token_in: &Address,
    amount_in: i128,
) {
    let entry = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: token_in.clone(),
            fn_name: symbol_short!("transfer"),
            args: (
                env.current_contract_address(),
                router_addr.clone(),
                amount_in,
            )
                .into_val(env),
        },
        sub_invocations: Vec::new(env),
    });
    env.authorize_as_current_contract(vec![env, entry]);
}
