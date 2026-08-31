pub mod budget;
pub mod tasks;

use anyhow::Result;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::classify::{classify_persistent, contract_label, KeyClass};
use crate::config::KeeperConfig;
use crate::discovery::{snapshot, ContractIds};
use crate::metrics::Metrics;
use crate::signer::Ed25519Signer;
use crate::stellar::client::RpcClient;
use crate::stellar::tx::{
    simulate_job, submit_with_sim, SimReport, SubmitOutcome, TxContext, TxJob,
};

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
    record_snapshot_metrics(metrics, &snap, ids, cfg.safety_margin_ledgers());

    let safety = cfg.safety_margin_ledgers();
    let restore_jobs = plan_restores(&snap, safety)?;
    let extend_jobs = plan_extends(&snap, safety)?;

    let restored = restored_keys(&restore_jobs);
    metrics.entries_archived.set(restored.len() as i64);
    let post_restore_extends = plan_extends_for_keys(&restored)?;

    let mut budget = TickBudget::new(cfg.schedule.max_txs_per_tick);
    let ctx = tx_context(cfg, client, signer);

    drive_jobs(&ctx, metrics, restore_jobs, dry_run, "ttl", &mut budget).await?;
    let mut extends = extend_jobs;
    if dry_run {
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
    record_snapshot_metrics(metrics, &snap, ids, cfg.safety_margin_ledgers());

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

fn record_snapshot_metrics(
    metrics: &Metrics,
    snap: &crate::discovery::DiscoverySnapshot,
    ids: &ContractIds,
    safety_ledgers: u32,
) {
    metrics.max_account_id.set(snap.max_account_id as i64);
    metrics.current_ledger.set(snap.current_ledger as i64);
    metrics.safety_margin_ledgers.set(safety_ledgers as i64);
    metrics.last_tick_timestamp_seconds.set(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    );
    publish_storage_gauges(metrics, snap, ids);
}

/// Classifies one entry into the state the dashboard reports.
///
/// The four states are distinct problems, and collapsing them loses the signal:
/// `never_created` in bulk is what a wrong key encoding looks like, while a
/// single `archived` row is real damage. `expired` mirrors what
/// `keeper_entries_archived` counts — the entry is still readable but its TTL
/// has lapsed, which is the case the keeper restores.
///
/// A key that was never written and one whose entry has been fully evicted are
/// told apart by whether the RPC still returns TTL metadata without a value.
/// That distinction is the RPC's to make; if it stops returning the TTL for an
/// evicted entry, such rows land in `never_created` and the bulk-vs-single
/// reading above is what separates them.
fn entry_state(
    row: &crate::stellar::client::LedgerEntryQuery,
    current_ledger: u32,
) -> &'static str {
    match (row.value.is_some(), row.live_until_ledger) {
        (true, Some(live_until)) if live_until < current_ledger => "expired",
        (true, _) => "live",
        (false, Some(_)) => "archived",
        (false, None) => "never_created",
    }
}

/// Publishes the per-`(contract, group)` TTL and entry-count gauges.
///
/// Both families are reset first, so a group that empties between ticks drops
/// its series instead of leaving a stale value stranded on the dashboard.
fn publish_storage_gauges(
    metrics: &Metrics,
    snap: &crate::discovery::DiscoverySnapshot,
    ids: &ContractIds,
) {
    metrics.entry_ttl_ledgers_min.reset();
    metrics.entries.reset();

    let mut min_ttl: BTreeMap<(String, &'static str), u32> = BTreeMap::new();
    let mut counts: BTreeMap<(String, &'static str, &'static str), i64> = BTreeMap::new();

    let classified = snap
        .persistent_entries
        .iter()
        .map(|row| {
            (
                row,
                classify_persistent(row, &ids.controller, ids.governance.as_ref()),
            )
        })
        .chain(
            snap.instance_entries
                .iter()
                .map(|row| (row, KeyClass::Instance)),
        )
        .chain(
            snap.wasm_code_entries
                .iter()
                .map(|row| (row, KeyClass::WasmCode)),
        );

    for (row, class) in classified {
        let contract = contract_label(
            &row.key,
            ids,
            snap.pool_id.as_ref(),
            snap.position_nft_id.as_ref(),
        );
        let group = class.label();
        let state = entry_state(row, snap.current_ledger);
        *counts.entry((contract.clone(), group, state)).or_insert(0) += 1;

        // Only a live entry has a meaningful TTL. Folding an absent one in as
        // zero would peg the group's minimum at zero and mask the real pacing
        // item behind a permanent false alarm.
        if row.value.is_some() {
            if let Some(live_until) = row.live_until_ledger {
                let remaining = live_until.saturating_sub(snap.current_ledger);
                min_ttl
                    .entry((contract, group))
                    .and_modify(|m| *m = (*m).min(remaining))
                    .or_insert(remaining);
            }
        }
    }

    for ((contract, group), remaining) in &min_ttl {
        metrics
            .entry_ttl_ledgers_min
            .with_label_values(&[contract, group])
            .set(i64::from(*remaining));
    }
    for ((contract, group, state), n) in &counts {
        metrics
            .entries
            .with_label_values(&[contract, group, state])
            .set(*n);
    }
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
                    metrics
                        .sim_resource_fee_stroops
                        .with_label_values(&[kind.as_str()])
                        .set(resource_fee as f64);
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

#[cfg(test)]
mod tests {
    use super::entry_state;
    use crate::stellar::client::LedgerEntryQuery;
    use stellar_xdr::{ContractDataDurability, LedgerEntryData, LedgerKey, ScAddress, ScVal};

    const NOW: u32 = 1_000;

    fn row(value: bool, live_until: Option<u32>) -> LedgerEntryQuery {
        LedgerEntryQuery {
            key: LedgerKey::ContractData(stellar_xdr::LedgerKeyContractData {
                contract: ScAddress::Contract(stellar_xdr::ContractId(stellar_xdr::Hash(
                    [0u8; 32],
                ))),
                key: ScVal::Void,
                durability: ContractDataDurability::Persistent,
            }),
            value: value.then_some(LedgerEntryData::ContractData(
                stellar_xdr::ContractDataEntry {
                    ext: stellar_xdr::ExtensionPoint::V0,
                    contract: ScAddress::Contract(stellar_xdr::ContractId(stellar_xdr::Hash(
                        [0u8; 32],
                    ))),
                    key: ScVal::Void,
                    durability: ContractDataDurability::Persistent,
                    val: ScVal::Void,
                },
            )),
            live_until_ledger: live_until,
        }
    }

    /// The four states are four different problems. Collapsing them is what made
    /// `controller/per_user absent=69` read like damage when every one of those
    /// keys had simply never been written, and what let a whole contract's rows
    /// read "absent" while the real cause was a key encoding that matched
    /// nothing.
    #[test]
    fn entry_states_separate_the_four_cases() {
        assert_eq!(entry_state(&row(true, Some(NOW + 500)), NOW), "live");
        assert_eq!(entry_state(&row(true, Some(NOW - 1)), NOW), "expired");
        assert_eq!(entry_state(&row(false, Some(NOW + 500)), NOW), "archived");
        assert_eq!(entry_state(&row(false, None), NOW), "never_created");
    }

    /// An entry whose TTL lapses exactly at the current ledger is still live —
    /// the bound is exclusive, matching `policy::classify`.
    #[test]
    fn ttl_expiring_at_the_current_ledger_is_still_live() {
        assert_eq!(entry_state(&row(true, Some(NOW)), NOW), "live");
    }
}
