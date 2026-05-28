//! `InvokeHostFunction` op builders for the controller's keeper endpoints
//! that the keeper still needs.
//!
//! After the permissionless-TTL refactor only `update_indexes` remains —
//! the only off-chain entry point that mutates pool state and therefore
//! still requires the KEEPER role.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{
    ContractId, Hash, HostFunction, InvokeContractArgs, InvokeHostFunctionOp, Operation,
    OperationBody, ScAddress, ScSymbol, ScVal, ScVec, StringM, VecM,
};

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
    let pk = stellar_strkey::ed25519::PublicKey::from_string(g_strkey)
        .map_err(|e| anyhow!("invalid signer G... address: {e}"))?;
    Ok(ScVal::Address(ScAddress::Account(
        stellar_xdr::curr::AccountId(
            stellar_xdr::curr::PublicKey::PublicKeyTypeEd25519(stellar_xdr::curr::Uint256(pk.0)),
        ),
    )))
}

fn into_vec_m<T>(items: Vec<T>) -> Result<VecM<T>> {
    items.try_into().map_err(|_| anyhow!("ScVec capacity exceeded"))
}
