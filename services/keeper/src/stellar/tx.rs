//! Build → simulate → patch → sign → submit.
//!
//! All keeper transactions share one shape: a single operation authorized by
//! the source account only. `update_indexes` relies on `caller.require_auth()`
//! and `ExtendFootprintTtl` needs no contract auth at all. Simulation rejects
//! anything that would require a richer auth shape, so the keeper never
//! silently issues a transaction whose authorization it did not anticipate.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use stellar_rpc_client::{Client as RpcInner, GetTransactionResponse};
use stellar_xdr::curr::{
    DecoratedSignature, Memo, Operation, Preconditions, SequenceNumber, Signature, SignatureHint,
    SorobanAuthorizationEntry, SorobanCredentials, SorobanResources, SorobanTransactionData,
    Transaction, TransactionEnvelope, TransactionExt, TransactionV1Envelope, VecM,
};
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::signer::Ed25519Signer;
use crate::stellar::client::{muxed_account_from_strkey, RpcClient};

/// One concrete unit of work that the submitter can ship as a single tx.
#[derive(Debug, Clone)]
pub struct TxJob {
    pub kind: TxKind,
    pub op: Operation,
    /// Footprint seed handed to the simulator. For `InvokeHostFunction` jobs
    /// the simulator infers and returns the real footprint, so this is
    /// `None`. For `ExtendFootprintTtl` the read-only keys must be declared
    /// up front — pass `Some(soroban_data_with_read_only_set)`.
    pub initial_soroban_data: Option<SorobanTransactionData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxKind {
    ExtendFootprintTtl,
    UpdateIndexes,
}

impl TxKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ExtendFootprintTtl => "extend_footprint_ttl",
            Self::UpdateIndexes => "update_indexes",
        }
    }
}

/// Outcome reported back to the scheduler so it can record metrics and
/// decide whether to advance the local sequence cache.
#[derive(Debug)]
pub enum SubmitOutcome {
    Success(Box<GetTransactionResponse>),
    SkippedSimError(String),
    Retriable(String),
    Failed(String),
}

pub struct TxContext<'a> {
    pub client: &'a RpcClient,
    pub signer: &'a Ed25519Signer,
    pub network_passphrase: &'a str,
    pub base_fee_stroops: u32,
    pub resource_fee_multiplier: f64,
    pub poll_timeout_seconds: u32,
}

/// Full pipeline for a single tx job. Returns `SubmitOutcome` so the caller
/// can decide whether to advance the local sequence.
pub async fn submit_with_sim(ctx: &TxContext<'_>, job: TxJob) -> Result<SubmitOutcome> {
    let TxJob {
        kind,
        op,
        initial_soroban_data,
    } = job;

    let source_strkey = ctx.signer.public_key_strkey();
    let source_seq = ctx
        .client
        .get_account_sequence(&source_strkey)
        .await
        .with_context(|| format!("look up sequence for {source_strkey}"))?;

    let envelope = build_envelope(
        &source_strkey,
        source_seq.saturating_add(1),
        ctx.base_fee_stroops,
        op,
        initial_soroban_data,
    )?;

    // The RPC rejects `authMode` on non-invoke operations
    // ("cannot set authMode with non-InvokeHostFunction operations"), so we
    // only pass an auth mode for actual contract invocations.
    let auth_mode = match kind {
        TxKind::ExtendFootprintTtl => None,
        _ => Some(stellar_rpc_client::AuthMode::Enforce),
    };
    let sim = ctx
        .client
        .inner()
        .simulate_transaction_envelope(&envelope, auth_mode)
        .await
        .context("simulate_transaction_envelope")?;

    if let Some(err) = sim.error {
        warn!(target: "keeper.tx", kind = %kind.as_str(), error = %err, "simulation rejected job");
        return Ok(SubmitOutcome::SkippedSimError(err));
    }

    let soroban_data = sim
        .transaction_data()
        .map_err(|e| anyhow!("decode simulation transaction_data: {e}"))?;

    if matches!(kind, TxKind::ExtendFootprintTtl) {
        debug!(
            target: "keeper.tx",
            kind = %kind.as_str(),
            instructions = soroban_data.resources.instructions,
            disk_read_bytes = soroban_data.resources.disk_read_bytes,
            write_bytes = soroban_data.resources.write_bytes,
            ro_keys = soroban_data.resources.footprint.read_only.len(),
            rw_keys = soroban_data.resources.footprint.read_write.len(),
            resource_fee = soroban_data.resource_fee,
            "sim returned soroban_data for extend"
        );
    }

    // Enforce expectation: only source-account auth is acceptable.
    enforce_source_account_auth(&sim, kind).context("auth shape check")?;

    let resource_fee = soroban_data.resource_fee;
    let bumped_resource_fee =
        bump_resource_fee(resource_fee, ctx.resource_fee_multiplier).max(sim.min_resource_fee as i64);
    let mut patched_data = soroban_data;
    patched_data.resource_fee = bumped_resource_fee;

    let total_fee: u32 = (ctx.base_fee_stroops as u64)
        .saturating_add(bumped_resource_fee.max(0) as u64)
        .try_into()
        .map_err(|_| anyhow!("computed fee exceeds u32"))?;

    let final_envelope = finalize_envelope(envelope, total_fee, patched_data)?;
    let signed = sign_envelope(final_envelope, ctx.signer, ctx.network_passphrase)?;

    debug!(
        target: "keeper.tx",
        kind = %kind.as_str(),
        fee_stroops = total_fee,
        resource_fee = bumped_resource_fee,
        "submitting"
    );

    submit_polling(ctx.client.inner(), &signed, ctx.poll_timeout_seconds, kind).await
}

fn enforce_source_account_auth(
    sim: &stellar_rpc_client::SimulateTransactionResponse,
    kind: TxKind,
) -> Result<()> {
    let results = sim
        .results()
        .map_err(|e| anyhow!("decode sim results: {e}"))?;

    // ExtendFootprintTTL has no host-function result.
    if matches!(kind, TxKind::ExtendFootprintTtl) {
        if !results.is_empty() {
            warn!(target: "keeper.tx", "extend_footprint_ttl unexpectedly returned host-function results — ignoring");
        }
        return Ok(());
    }

    if results.is_empty() {
        bail!("simulation produced no host-function result for {}", kind.as_str());
    }
    for r in &results {
        for entry in &r.auth {
            let SorobanAuthorizationEntry { credentials, .. } = entry;
            if !matches!(credentials, SorobanCredentials::SourceAccount) {
                bail!(
                    "{} requires non-source-account auth (credentials kind = {:?}); skipping",
                    kind.as_str(),
                    credentials
                );
            }
        }
    }
    Ok(())
}

fn bump_resource_fee(resource_fee: i64, multiplier: f64) -> i64 {
    let bumped = (resource_fee as f64 * multiplier).ceil();
    if bumped.is_finite() && bumped >= 0.0 && bumped <= i64::MAX as f64 {
        bumped as i64
    } else {
        resource_fee
    }
}

fn build_envelope(
    source_strkey: &str,
    seq_num: i64,
    fee: u32,
    op: Operation,
    seed_soroban_data: Option<SorobanTransactionData>,
) -> Result<TransactionEnvelope> {
    let source_account = muxed_account_from_strkey(source_strkey)?;

    let ops: VecM<Operation, 100> = vec![op]
        .try_into()
        .map_err(|_| anyhow!("too many ops for a tx"))?;

    let tx = Transaction {
        source_account,
        fee,
        seq_num: SequenceNumber(seq_num),
        cond: Preconditions::None,
        memo: Memo::None,
        operations: ops,
        ext: TransactionExt::V1(seed_soroban_data.unwrap_or_else(empty_soroban_data)),
    };

    Ok(TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures: VecM::default(),
    }))
}

fn finalize_envelope(
    envelope: TransactionEnvelope,
    total_fee: u32,
    soroban_data: SorobanTransactionData,
) -> Result<TransactionEnvelope> {
    let TransactionEnvelope::Tx(TransactionV1Envelope { mut tx, .. }) = envelope else {
        bail!("non-v1 envelope is not supported");
    };
    tx.fee = total_fee;
    tx.ext = TransactionExt::V1(soroban_data);
    Ok(TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures: VecM::default(),
    }))
}

pub(crate) fn empty_soroban_data() -> SorobanTransactionData {
    use stellar_xdr::curr::{LedgerFootprint, SorobanTransactionDataExt};
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

fn sign_envelope(
    envelope: TransactionEnvelope,
    signer: &Ed25519Signer,
    network_passphrase: &str,
) -> Result<TransactionEnvelope> {
    let TransactionEnvelope::Tx(TransactionV1Envelope { tx, .. }) = envelope else {
        bail!("non-v1 envelope is not supported");
    };

    let network_id: [u8; 32] = Sha256::digest(network_passphrase.as_bytes()).into();
    let tx_hash = tx
        .hash(network_id)
        .map_err(|e| anyhow!("hash transaction: {e}"))?;
    let sig_bytes = signer.sign(&tx_hash);

    let signature: Signature = sig_bytes
        .to_vec()
        .try_into()
        .map_err(|_| anyhow!("signature length unexpected"))?;
    let decorated = DecoratedSignature {
        hint: SignatureHint(signer.signature_hint()),
        signature,
    };
    let signatures: VecM<DecoratedSignature, 20> = vec![decorated]
        .try_into()
        .map_err(|_| anyhow!("too many signatures"))?;

    Ok(TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures,
    }))
}

async fn submit_polling(
    client: &RpcInner,
    envelope: &TransactionEnvelope,
    timeout_s: u32,
    kind: TxKind,
) -> Result<SubmitOutcome> {
    // `send_transaction_polling` runs its own poll loop with no upper bound, so
    // cap it here: a stuck submission must not pin the tick indefinitely. On
    // timeout the next tick retries with a refreshed sequence, and TTL extends
    // are idempotent, so a retry is always safe.
    let poll = match timeout(
        Duration::from_secs(timeout_s.max(1) as u64),
        client.send_transaction_polling(envelope),
    )
    .await
    {
        Ok(result) => result,
        Err(_elapsed) => {
            warn!(target: "keeper.tx", kind = %kind.as_str(), timeout_s, "send_transaction_polling timed out");
            return Ok(SubmitOutcome::Retriable(format!(
                "submission poll exceeded {timeout_s}s"
            )));
        }
    };

    let resp = match poll {
        Ok(r) => r,
        Err(e) => {
            // Network or transport failure — caller should retry with refreshed
            // sequence.
            warn!(target: "keeper.tx", kind = %kind.as_str(), error = %e, "send_transaction_polling failed");
            return Ok(SubmitOutcome::Retriable(e.to_string()));
        }
    };

    match resp.status.as_str() {
        "SUCCESS" => Ok(SubmitOutcome::Success(Box::new(resp))),
        "NOT_FOUND" => Ok(SubmitOutcome::Retriable(
            "polling completed without terminal status".into(),
        )),
        "FAILED" => {
            warn!(
                target: "keeper.tx",
                kind = %kind.as_str(),
                "tx FAILED on-chain: {:?}",
                resp.result
            );
            Ok(SubmitOutcome::Failed(format!(
                "on-chain FAILED status: {:?}",
                resp.result
            )))
        }
        other => Ok(SubmitOutcome::Failed(format!("unknown status {other}"))),
    }
}
