use anyhow::{anyhow, Result};
use stellar_xdr::{
    ContractId, Hash, HostFunction, InvokeContractArgs, InvokeHostFunctionOp, Operation,
    OperationBody, ScAddress, ScSymbol, ScVal, ScVec, SorobanAuthorizationEntry,
    SorobanAuthorizedFunction, SorobanAuthorizedInvocation, SorobanCredentials, StringM, VecM,
};

use crate::keys::{hub_asset_key_sc_val, HubAssetKey};
use crate::stellar::client::account_id_from_strkey;
use crate::stellar::tx::{TxJob, TxKind};

pub fn update_indexes(
    controller_id: &[u8; 32],
    caller_strkey: &str,
    assets: &[HubAssetKey],
) -> Result<TxJob> {
    let caller = ScVal::Address(ScAddress::Account(account_id_from_strkey(caller_strkey)?));
    let assets_vec: VecM<ScVal> = assets
        .iter()
        .map(hub_asset_key_sc_val)
        .collect::<Result<Vec<_>>>()?
        .try_into()
        .map_err(|_| anyhow!("ScVec capacity exceeded"))?;
    let args_vec: VecM<ScVal> = vec![caller, ScVal::Vec(Some(ScVec(assets_vec)))]
        .try_into()
        .map_err(|_| anyhow!("too many args"))?;
    Ok(TxJob {
        kind: TxKind::UpdateIndexes,
        op: invoke_op(controller_id, "update_indexes", args_vec)?,
        initial_soroban_data: None,
    })
}

fn invoke_op(contract_id: &[u8; 32], function: &str, args: VecM<ScVal>) -> Result<Operation> {
    let function_name = ScSymbol(
        StringM::<32>::try_from(function)
            .map_err(|_| anyhow!("function name {function} > 32 bytes"))?,
    );
    let invoke_args = InvokeContractArgs {
        contract_address: ScAddress::Contract(ContractId(Hash(*contract_id))),
        function_name,
        args,
    };
    let source_auth = SorobanAuthorizationEntry {
        credentials: SorobanCredentials::SourceAccount,
        root_invocation: SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(invoke_args.clone()),
            sub_invocations: VecM::default(),
        },
    };
    Ok(Operation {
        source_account: None,
        body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
            host_function: HostFunction::InvokeContract(invoke_args),
            auth: vec![source_auth]
                .try_into()
                .map_err(|_| anyhow!("auth vector capacity exceeded"))?,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_indexes_carries_source_account_auth_for_the_same_call() {
        let job = update_indexes(
            &[7u8; 32],
            "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6",
            &[],
        )
        .unwrap();
        let OperationBody::InvokeHostFunction(op) = job.op.body else {
            panic!("not an invoke op");
        };
        let HostFunction::InvokeContract(args) = op.host_function else {
            panic!("not a contract call");
        };
        assert_eq!(op.auth.len(), 1);
        let entry = &op.auth[0];
        assert!(matches!(
            entry.credentials,
            SorobanCredentials::SourceAccount
        ));
        assert_eq!(
            entry.root_invocation.function,
            SorobanAuthorizedFunction::ContractFn(args)
        );
        assert!(entry.root_invocation.sub_invocations.is_empty());
    }
}
