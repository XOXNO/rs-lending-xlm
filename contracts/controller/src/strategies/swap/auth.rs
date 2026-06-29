//! Router token-pull preauthorization.

use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{symbol_short, Address, Env, IntoVal, Vec};

pub(super) fn pre_authorize_router_pull(
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
    env.authorize_as_current_contract(soroban_sdk::vec![env, entry]);
}
