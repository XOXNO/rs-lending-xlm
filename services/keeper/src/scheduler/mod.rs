//! Scheduler tick loop and submitter.

pub mod budget;
pub mod tasks;

use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::KeeperConfig;
use crate::discovery::{snapshot, ContractIds};
use crate::metrics::Metrics;
use crate::signer::Ed25519Signer;
use crate::stellar::tx::{simulate_job, submit_with_sim, SimReport, SubmitOutcome, TxContext};
use crate::stellar::{RpcClient, TxJob};

use self::budget::TickBudget;
use self::tasks::{
    plan_extends, plan_extends_for_keys, plan_index_refresh, plan_restores, restored_keys,
};

pub struct SchedulerHandle {
    pub ttl_task: tokio::task::JoinHandle<()>,
    pub index_task: Option<tokio::task::JoinHandle<()>>,
}

pub async fn run(
    cfg: Arc<KeeperConfig>,
    client: Arc<RpcClient>,
    signer: Arc<Ed25519Signer>,
    metrics: Arc<Metrics>,
    cancel: CancellationToken,
    dry_run: bool,
) -> Result<SchedulerHandle> {
    // Parse contract ids once; the loops below run them every tick without
    // re-parsing or cloning the config strkeys.
    let ids = ContractIds::resolve(&cfg.contracts)?;

    let ttl = spawn_ttl_loop(
        cfg.clone(),
        client.clone(),
        signer.clone(),
        metrics.clone(),
        cancel.clone(),
        dry_run,
        ids,
    );
    let index = if cfg.schedule.enable_index_refresh {
        Some(spawn_index_loop(
            cfg, client, signer, metrics, cancel, dry_run, ids,
        ))
    } else {
        None
    };
    Ok(SchedulerHandle {
        ttl_task: ttl,
        index_task: index,
    })
}

fn spawn_ttl_loop(
    cfg: Arc<KeeperConfig>,
    client: Arc<RpcClient>,
    signer: Arc<Ed25519Signer>,
    metrics: Arc<Metrics>,
    cancel: CancellationToken,
    dry_run: bool,
    ids: ContractIds,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(cfg.schedule.ttl_tick_seconds.max(1)));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // Burn the immediate tick; sleep for the configured cadence before the
        // first sweep so the rest of the boot sequence (axum, metrics) can
        // settle.
        tick.tick().await;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!(target: "keeper.scheduler", "ttl loop cancelled");
                    return;
                }
                _ = tick.tick() => {
                    if let Err(e) = run_ttl_tick(&cfg, &client, &signer, &metrics, dry_run, &ids).await {
                        error!(target: "keeper.scheduler", error = ?e, "ttl tick failed");
                        metrics.tick_failed.with_label_values(&["ttl"]).inc();
                    }
                }
            }
        }
    })
}

fn spawn_index_loop(
    cfg: Arc<KeeperConfig>,
    client: Arc<RpcClient>,
    signer: Arc<Ed25519Signer>,
    metrics: Arc<Metrics>,
    cancel: CancellationToken,
    dry_run: bool,
    ids: ContractIds,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(cfg.schedule.index_tick_seconds.max(1)));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        tick.tick().await;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!(target: "keeper.scheduler", "index loop cancelled");
                    return;
                }
                _ = tick.tick() => {
                    if let Err(e) = run_index_tick(&cfg, &client, &signer, &metrics, dry_run, &ids).await {
                        error!(target: "keeper.scheduler", error = ?e, "index tick failed");
                        metrics.tick_failed.with_label_values(&["index"]).inc();
                    }
                }
            }
        }
    })
}

async fn run_ttl_tick(
    cfg: &KeeperConfig,
    client: &RpcClient,
    signer: &Ed25519Signer,
    metrics: &Metrics,
    dry_run: bool,
    ids: &ContractIds,
) -> Result<()> {
    let snap = snapshot(client, ids, &cfg.contracts, &cfg.schedule).await?;
    record_snapshot_metrics(metrics, &snap);

    let safety = cfg.safety_margin_ledgers();
    let restore_jobs = plan_restores(&snap, safety)?;
    let extend_jobs = plan_extends(&snap, safety)?;

    // Freshly-restored entries come back at the network-minimum TTL, so extend
    // them to the cap the same tick — but only after the restores land, since an
    // extend over a still-archived entry is rejected.
    let restored = restored_keys(&restore_jobs);
    metrics.entries_archived.set(restored.len() as i64);
    let post_restore_extends = plan_extends_for_keys(&restored)?;

    // One budget for the whole tick — restores and extends share the cap so a
    // tick never submits more than `max_txs_per_tick` transactions in total;
    // jobs over the cap retry next tick.
    let mut budget = TickBudget::new(cfg.schedule.max_txs_per_tick);
    let ctx = tx_context(cfg, client, signer);

    // Restores first (they unblock the protocol), then in-margin extends.
    drive_jobs(&ctx, metrics, restore_jobs, dry_run, "ttl", &mut budget).await?;
    let mut extends = extend_jobs;
    if dry_run {
        // Post-restore extends would fail simulation while the entry is still
        // archived, so report them instead of simulating.
        if !post_restore_extends.is_empty() {
            info!(
                target: "keeper.scheduler",
                restored = restored.len(),
                "[dry-run] would extend restored keys after restore lands (not simulated — would fail pre-restore)"
            );
        }
    } else {
        extends.extend(post_restore_extends);
    }
    drive_jobs(&ctx, metrics, extends, dry_run, "ttl", &mut budget).await
}

async fn run_index_tick(
    cfg: &KeeperConfig,
    client: &RpcClient,
    signer: &Ed25519Signer,
    metrics: &Metrics,
    dry_run: bool,
    ids: &ContractIds,
) -> Result<()> {
    let snap = snapshot(client, ids, &cfg.contracts, &cfg.schedule).await?;
    record_snapshot_metrics(metrics, &snap);

    if snap.assets.is_empty() {
        return Ok(());
    }
    let jobs = plan_index_refresh(
        &ids.controller,
        &signer.public_key_strkey(),
        &snap.assets,
        cfg.schedule.asset_chunk,
    )?;
    let mut budget = TickBudget::new(cfg.schedule.max_txs_per_tick);
    let ctx = tx_context(cfg, client, signer);
    drive_jobs(&ctx, metrics, jobs, dry_run, "index", &mut budget).await
}

/// Build the per-tx submission context from config + connections.
fn tx_context<'a>(
    cfg: &'a KeeperConfig,
    client: &'a RpcClient,
    signer: &'a Ed25519Signer,
) -> TxContext<'a> {
    TxContext {
        client,
        signer,
        network_passphrase: &cfg.rpc.passphrase,
        base_fee_stroops: cfg.fees.base_fee_stroops,
        resource_fee_multiplier: cfg.fees.resource_fee_multiplier,
        poll_timeout_seconds: cfg.rpc.timeout_seconds as u32,
    }
}

fn record_snapshot_metrics(metrics: &Metrics, snap: &crate::discovery::DiscoverySnapshot) {
    metrics.account_nonce.set(snap.account_nonce as i64);
}

async fn drive_jobs(
    ctx: &TxContext<'_>,
    metrics: &Metrics,
    jobs: Vec<TxJob>,
    dry_run: bool,
    loop_label: &str,
    budget: &mut TickBudget,
) -> Result<()> {
    metrics
        .jobs_planned
        .with_label_values(&[loop_label])
        .inc_by(jobs.len() as u64);

    for job in jobs {
        if !budget.try_spend() {
            warn!(
                target: "keeper.scheduler",
                loop_label,
                spent = budget.spent(),
                "tick budget exhausted; deferring remaining jobs to next tick"
            );
            break;
        }
        let kind = job.kind;
        if dry_run {
            match simulate_job(ctx, &job).await {
                Ok(SimReport::Ok {
                    resource_fee,
                    read_only,
                    read_write,
                }) => {
                    info!(
                        target: "keeper.scheduler",
                        kind = kind.as_str(),
                        resource_fee,
                        read_only,
                        read_write,
                        "[dry-run] sim ok — would submit"
                    );
                    metrics
                        .tx_total
                        .with_label_values(&[kind.as_str(), "dry_run_ok"])
                        .inc();
                }
                Ok(SimReport::Rejected(reason)) => {
                    warn!(
                        target: "keeper.scheduler",
                        kind = kind.as_str(),
                        %reason,
                        "[dry-run] sim REJECTED"
                    );
                    metrics
                        .sim_failures
                        .with_label_values(&[kind.as_str(), classify_reason(&reason)])
                        .inc();
                }
                Err(e) => {
                    error!(target: "keeper.scheduler", kind = kind.as_str(), error = ?e, "[dry-run] sim pipeline error");
                }
            }
            continue;
        }
        match submit_with_sim(ctx, job).await {
            Ok(SubmitOutcome::Success(_)) => {
                metrics
                    .tx_total
                    .with_label_values(&[kind.as_str(), "success"])
                    .inc();
            }
            Ok(SubmitOutcome::SkippedSimError(reason)) => {
                metrics
                    .sim_failures
                    .with_label_values(&[kind.as_str(), classify_reason(&reason)])
                    .inc();
            }
            Ok(SubmitOutcome::Retriable(reason)) => {
                warn!(target: "keeper.scheduler", kind = kind.as_str(), %reason, "retriable failure");
                metrics
                    .tx_total
                    .with_label_values(&[kind.as_str(), "retriable"])
                    .inc();
            }
            Ok(SubmitOutcome::Failed(reason)) => {
                error!(target: "keeper.scheduler", kind = kind.as_str(), %reason, "tx failed");
                metrics
                    .tx_total
                    .with_label_values(&[kind.as_str(), "failed"])
                    .inc();
            }
            Err(e) => {
                error!(target: "keeper.scheduler", kind = kind.as_str(), error = ?e, "submitter pipeline error");
                metrics
                    .tx_total
                    .with_label_values(&[kind.as_str(), "error"])
                    .inc();
            }
        }
    }
    Ok(())
}

fn classify_reason(msg: &str) -> &'static str {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("unauthor") || lower.contains("role") {
        "unauthorized"
    } else if lower.contains("budget") || lower.contains("instruction") {
        "budget"
    } else if lower.contains("archiv") {
        "archived"
    } else {
        "other"
    }
}
