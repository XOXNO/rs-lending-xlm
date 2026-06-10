//! Transaction build, simulation, signing, and submission.

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
    /// Optional footprint seed for simulation.
    pub initial_soroban_data: Option<SorobanTransactionData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxKind {
    ExtendFootprintTtl,
    RestoreFootprint,
    UpdateIndexes,
}

impl TxKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ExtendFootprintTtl => "extend_footprint_ttl",
            Self::RestoreFootprint => "restore_footprint",
            Self::UpdateIndexes => "update_indexes",
        }
    }

    /// True for permissionless footprint operations.
    fn is_footprint_op(self) -> bool {
        matches!(self, Self::ExtendFootprintTtl | Self::RestoreFootprint)
    }
}

/// Submitted transaction outcome.
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

/// Builds, simulates, signs, submits, and polls one job.
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

    let (envelope, sim) = build_and_simulate(
        ctx,
        kind,
        op,
        initial_soroban_data,
        source_seq.saturating_add(1),
    )
    .await?;

    if let Some(err) = sim.error {
        warn!(target: "keeper.tx", kind = %kind.as_str(), error = %err, "simulation rejected job");
        return Ok(SubmitOutcome::SkippedSimError(err));
    }

    let soroban_data = sim
        .transaction_data()
        .map_err(|e| anyhow!("decode simulation transaction_data: {e}"))?;

    if kind.is_footprint_op() {
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
    let bumped_resource_fee = bump_resource_fee(resource_fee, ctx.resource_fee_multiplier)
        .max(sim.min_resource_fee as i64);
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

/// Outcome of simulating a job without submitting it.
#[derive(Debug)]
pub enum SimReport {
    Ok {
        resource_fee: i64,
        read_only: usize,
        read_write: usize,
    },
    Rejected(String),
}

/// Builds an envelope and simulates it.
async fn build_and_simulate(
    ctx: &TxContext<'_>,
    kind: TxKind,
    op: Operation,
    initial_soroban_data: Option<SorobanTransactionData>,
    seq_num: i64,
) -> Result<(
    TransactionEnvelope,
    stellar_rpc_client::SimulateTransactionResponse,
)> {
    let source_strkey = ctx.signer.public_key_strkey();
    let envelope = build_envelope(
        &source_strkey,
        seq_num,
        ctx.base_fee_stroops,
        op,
        initial_soroban_data,
    )?;
    let auth_mode = if kind.is_footprint_op() {
        None
    } else {
        Some(stellar_rpc_client::AuthMode::Enforce)
    };
    let sim = ctx
        .client
        .inner()
        .simulate_transaction_envelope(&envelope, auth_mode)
        .await
        .context("simulate_transaction_envelope")?;
    Ok((envelope, sim))
}

/// Simulates a job without submitting it.
pub async fn simulate_job(ctx: &TxContext<'_>, job: &TxJob) -> Result<SimReport> {
    let (_envelope, sim) = build_and_simulate(
        ctx,
        job.kind,
        job.op.clone(),
        job.initial_soroban_data.clone(),
        0,
    )
    .await?;

    if let Some(err) = sim.error {
        return Ok(SimReport::Rejected(err));
    }
    let data = sim
        .transaction_data()
        .map_err(|e| anyhow!("decode simulation transaction_data: {e}"))?;
    Ok(SimReport::Ok {
        resource_fee: data.resource_fee,
        read_only: data.resources.footprint.read_only.len(),
        read_write: data.resources.footprint.read_write.len(),
    })
}

fn enforce_source_account_auth(
    sim: &stellar_rpc_client::SimulateTransactionResponse,
    kind: TxKind,
) -> Result<()> {
    let results = sim
        .results()
        .map_err(|e| anyhow!("decode sim results: {e}"))?;

    // Footprint ops (extend / restore) carry no host-function result.
    if kind.is_footprint_op() {
        if !results.is_empty() {
            warn!(target: "keeper.tx", kind = %kind.as_str(), "footprint op unexpectedly returned host-function results — ignoring");
        }
        return Ok(());
    }

    if results.is_empty() {
        bail!(
            "simulation produced no host-function result for {}",
            kind.as_str()
        );
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

pub(crate) fn build_envelope(
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
