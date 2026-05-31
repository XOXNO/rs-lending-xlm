//! `InvokeHostFunction` op builders for the controller's keeper endpoints.
//!
//! `update_indexes` is the only such endpoint the keeper calls: it advances
//! pool interest accrual and is the one keeper operation that mutates state,
//! so it requires the signer to hold the on-chain KEEPER role. (TTL bumping is
//! permissionless and is built in `ttl.rs`, not here.)

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{
    ContractId, Hash, HostFunction, InvokeContractArgs, InvokeHostFunctionOp, Operation,
    OperationBody, ScAddress, ScSymbol, ScVal, ScVec, StringM, VecM,
};

use crate::stellar::client::account_id_from_strkey;
use crate::stellar::tx::TxKind;
use crate::stellar::TxJob;

/// `controller.update_indexes(caller, assets)`.
pub fn update_indexes(
    controller_id: &[u8; 32],
    caller_strkey: &str,
    assets: &[[u8; 32]],
) -> Result<TxJob> {
    let caller = caller_address(caller_strkey)?;
    let assets_vec: Vec<ScVal> = assets
        .iter()
        .map(|a| ScVal::Address(ScAddress::Contract(ContractId(Hash(*a)))))
        .collect();
    let args_vec: VecM<ScVal> =
        vec![caller, ScVal::Vec(Some(ScVec(into_vec_m(assets_vec)?)))]
            .try_into()
            .map_err(|_| anyhow!("too many args"))?;
    Ok(TxJob {
        kind: TxKind::UpdateIndexes,
        op: invoke_op(controller_id, "update_indexes", args_vec)?,
        initial_soroban_data: None,
    })
}

fn invoke_op(
    contract_id: &[u8; 32],
    function: &str,
    args: VecM<ScVal>,
) -> Result<Operation> {
    let function_name = ScSymbol(
        StringM::<32>::try_from(function)
            .map_err(|_| anyhow!("function name {function} > 32 bytes"))?,
    );
    let invoke_args = InvokeContractArgs {
        contract_address: ScAddress::Contract(ContractId(Hash(*contract_id))),
        function_name,
        args,
    };
    Ok(Operation {
        source_account: None,
        body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
            host_function: HostFunction::InvokeContract(invoke_args),
            auth: VecM::default(),
        }),
    })
}

fn caller_address(g_strkey: &str) -> Result<ScVal> {
    Ok(ScVal::Address(ScAddress::Account(account_id_from_strkey(
        g_strkey,
    )?)))
}

fn into_vec_m<T>(items: Vec<T>) -> Result<VecM<T>> {
    items.try_into().map_err(|_| anyhow!("ScVec capacity exceeded"))
}
