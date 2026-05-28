//! `InvokeHostFunction` op builders for the controller's keeper endpoints.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{
    ContractId, Hash, HostFunction, InvokeContractArgs, InvokeHostFunctionOp, Operation,
    OperationBody, ScAddress, ScSymbol, ScVal, ScVec, StringM, VecM,
};

use crate::stellar::tx::TxKind;
use crate::stellar::TxJob;

/// `controller.keepalive_shared_state(caller, assets)`.
pub fn keepalive_shared_state(
    controller_id: &[u8; 32],
    caller_strkey: &str,
    assets: &[[u8; 32]],
) -> Result<TxJob> {
    invoke_with_caller_and_assets(
        TxKind::KeepaliveShared,
        "keepalive_shared_state",
        controller_id,
        caller_strkey,
        assets,
    )
}

/// `controller.keepalive_pools(caller, assets)`.
pub fn keepalive_pools(
    controller_id: &[u8; 32],
    caller_strkey: &str,
    assets: &[[u8; 32]],
) -> Result<TxJob> {
    invoke_with_caller_and_assets(
        TxKind::KeepalivePools,
        "keepalive_pools",
        controller_id,
        caller_strkey,
        assets,
    )
}

/// `controller.keepalive_accounts(caller, account_ids)`.
pub fn keepalive_accounts(
    controller_id: &[u8; 32],
    caller_strkey: &str,
    ids: &[u64],
) -> Result<TxJob> {
    let caller = caller_address(caller_strkey)?;
    let ids_vec: Vec<ScVal> = ids.iter().copied().map(ScVal::U64).collect();
    let args_vec: VecM<ScVal> = vec![caller, ScVal::Vec(Some(ScVec(into_vec_m(ids_vec)?)))]
        .try_into()
        .map_err(|_| anyhow!("too many args"))?;
    Ok(TxJob {
        kind: TxKind::KeepaliveAccounts,
        op: invoke_op(controller_id, "keepalive_accounts", args_vec)?,
        initial_soroban_data: None,
    })
}

/// `controller.update_indexes(caller, assets)`.
pub fn update_indexes(
    controller_id: &[u8; 32],
    caller_strkey: &str,
    assets: &[[u8; 32]],
) -> Result<TxJob> {
    invoke_with_caller_and_assets(
        TxKind::UpdateIndexes,
        "update_indexes",
        controller_id,
        caller_strkey,
        assets,
    )
}

fn invoke_with_caller_and_assets(
    kind: TxKind,
    function: &str,
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
        kind,
        op: invoke_op(controller_id, function, args_vec)?,
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
