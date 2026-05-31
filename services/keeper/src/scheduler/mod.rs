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
use crate::stellar::tx::{submit_with_sim, SubmitOutcome, TxContext};
use crate::stellar::{RpcClient, TxJob};

use self::budget::TickBudget;
use self::tasks::{plan_extends, plan_index_refresh};

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
        Some(spawn_index_loop(cfg, client, signer, metrics, cancel, dry_run, ids))
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
    let snap = snapshot(client, ids, cfg.schedule.asset_chunk).await?;
    record_snapshot_metrics(metrics, &snap);

    let jobs = plan_extends(&snap, cfg.safety_margin_ledgers())?;
    drive_jobs(cfg, client, signer, metrics, jobs, dry_run, "ttl").await
}

async fn run_index_tick(
    cfg: &KeeperConfig,
    client: &RpcClient,
    signer: &Ed25519Signer,
    metrics: &Metrics,
    dry_run: bool,
    ids: &ContractIds,
) -> Result<()> {
    let snap = snapshot(client, ids, cfg.schedule.asset_chunk).await?;
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
    drive_jobs(cfg, client, signer, metrics, jobs, dry_run, "index").await
}

fn record_snapshot_metrics(metrics: &Metrics, snap: &crate::discovery::DiscoverySnapshot) {
    metrics.account_nonce.set(snap.account_nonce as i64);
    metrics.pools_listed.set(snap.assets.len() as i64);
}

async fn drive_jobs(
    cfg: &KeeperConfig,
    client: &RpcClient,
    signer: &Ed25519Signer,
    metrics: &Metrics,
    jobs: Vec<TxJob>,
    dry_run: bool,
    loop_label: &str,
) -> Result<()> {
    let mut budget = TickBudget::new(cfg.schedule.max_txs_per_tick);
    metrics
        .jobs_planned
        .with_label_values(&[loop_label])
        .inc_by(jobs.len() as u64);

    let ctx = TxContext {
        client,
        signer,
        network_passphrase: &cfg.rpc.passphrase,
        base_fee_stroops: cfg.fees.base_fee_stroops,
        resource_fee_multiplier: cfg.fees.resource_fee_multiplier,
        poll_timeout_seconds: cfg.rpc.timeout_seconds as u32,
    };

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
            info!(target: "keeper.scheduler", kind = kind.as_str(), "[dry-run] would submit");
            metrics
                .tx_total
                .with_label_values(&[kind.as_str(), "dry_run"])
                .inc();
            continue;
        }
        match submit_with_sim(&ctx, job).await {
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
