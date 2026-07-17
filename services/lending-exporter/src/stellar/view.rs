//! Read-only contract views via `simulateTransaction`.
//!
//! Builds op/envelope, simulates, decodes `sim.results()[0].xdr` as the return
//! ScVal. No signing or submit — throwaway all-zero source account.

use anyhow::{anyhow, Context};
use stellar_xdr::curr::{
    ContractId, Hash, HostFunction, InvokeContractArgs, InvokeHostFunctionOp, LedgerFootprint,
    Memo, MuxedAccount, Operation, OperationBody, Preconditions, ScAddress, ScSymbol, ScVal,
    SequenceNumber, SorobanResources, SorobanTransactionData, SorobanTransactionDataExt,
    StringM, Transaction, TransactionEnvelope, TransactionExt, TransactionV1Envelope, Uint256,
    VecM,
};
use thiserror::Error;

use crate::stellar::client::RpcClient;

/// View failure: contract revert (bucketable code) vs transport vs empty result.
#[derive(Debug, Error)]
pub enum ViewError {
    /// Contract panic during sim; string is RPC diagnostic for code bucketing.
    #[error("contract reverted: {0}")]
    Reverted(String),
    #[error("rpc error: {0}")]
    Rpc(#[from] anyhow::Error),
    #[error("simulation returned no result")]
    NoResult,
}

/// Simulate `contract.function(args)` read-only → return ScVal.
pub async fn simulate_view(
    client: &RpcClient,
    contract_id: &[u8; 32],
    function: &str,
    args: Vec<ScVal>,
) -> Result<ScVal, ViewError> {
    let op = invoke_op(contract_id, function, args)?;
    let envelope = read_only_envelope(op)?;

    let sim = client
        .inner()
        .simulate_transaction_envelope(&envelope, None)
        .await
        .context("simulate_transaction_envelope")?;

    if let Some(err) = sim.error {
        return Err(ViewError::Reverted(err));
    }

    let results = sim
        .results()
        .map_err(|e| ViewError::Rpc(anyhow!("decode sim results: {e}")))?;
    let first = results.into_iter().next().ok_or(ViewError::NoResult)?;
    Ok(first.xdr)
}

fn invoke_op(contract_id: &[u8; 32], function: &str, args: Vec<ScVal>) -> Result<Operation, ViewError> {
    let function_name = ScSymbol(
        StringM::<32>::try_from(function)
            .map_err(|_| ViewError::Rpc(anyhow!("function name {function} > 32 bytes")))?,
    );
    let args: VecM<ScVal> = args
        .try_into()
        .map_err(|_| ViewError::Rpc(anyhow!("too many view args")))?;
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

/// Single-op envelope from all-zero source; simulated only, never submitted.
fn read_only_envelope(op: Operation) -> Result<TransactionEnvelope, ViewError> {
    let ops: VecM<Operation, 100> = vec![op]
        .try_into()
        .map_err(|_| ViewError::Rpc(anyhow!("too many ops")))?;
    let tx = Transaction {
        source_account: MuxedAccount::Ed25519(Uint256([0u8; 32])),
        fee: 100,
        seq_num: SequenceNumber(0),
        cond: Preconditions::None,
        memo: Memo::None,
        operations: ops,
        ext: TransactionExt::V1(empty_soroban_data()),
    };
    Ok(TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures: VecM::default(),
    }))
}

fn empty_soroban_data() -> SorobanTransactionData {
    SorobanTransactionData {
        ext: SorobanTransactionDataExt::V0,
        resources: SorobanResources {
            footprint: LedgerFootprint {
                read_only: VecM::default(),
                read_write: VecM::default(),
            },
            instructions: 0,
            disk_read_bytes: 0,
            write_bytes: 0,
        },
        resource_fee: 0,
    }
}
