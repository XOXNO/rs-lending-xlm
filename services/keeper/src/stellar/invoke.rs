//! Controller invoke builders used by the keeper.
//!
//! Only `update_indexes` (advances pool accrual). Caller signs; no on-chain role.
//! TTL bumps are permissionless — see `ttl.rs`.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{
    ContractId, Hash, HostFunction, InvokeContractArgs, InvokeHostFunctionOp, Operation,
    OperationBody, ScAddress, ScSymbol, ScVal, ScVec, StringM, VecM,
};

use crate::keys::{hub_asset_key_sc_val, HubAssetKey};
use crate::stellar::client::account_id_from_strkey;
use crate::stellar::tx::TxKind;
use crate::stellar::TxJob;

/// `controller.update_indexes(caller, hub_assets)`.
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
    Ok(Operation {
        source_account: None,
        body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
            host_function: HostFunction::InvokeContract(invoke_args),
            auth: VecM::default(),
        }),
    })
}
