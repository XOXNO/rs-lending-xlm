//! Translate a discovery snapshot into a list of `TxJob`s the submitter can
//! run. Pure function — no I/O.
//!
//! v2 design: TTL bumping is **permissionless**, so the planner emits a
//! single stream of `ExtendFootprintTtl` ops covering every entry whose
//! `live_until` is below the safety margin (persistent storage, contract
//! instances, wasm code). The keeper's signer does not need any on-chain
//! role for these.
//!
//! The only operation that still requires the KEEPER role is
//! `update_indexes`, which mutates pool state (interest accrual) and is
//! the controller's only legitimate way to advance pool indexes from off
//! chain.

use anyhow::Result;
use stellar_xdr::curr::LedgerKey;
use tracing::debug;

use crate::config::ScheduleConfig;
use crate::discovery::DiscoverySnapshot;
use crate::policy::{needs_bump, BumpReason};
use crate::stellar::invoke::update_indexes;
use crate::stellar::ttl::extend_footprint_ttl;
use crate::stellar::TxJob;

pub struct PlannerInput<'a> {
    pub snapshot: &'a DiscoverySnapshot,
    pub schedule: &'a ScheduleConfig,
    pub controller_id: &'a [u8; 32],
    pub caller_strkey: &'a str,
    pub safety_ledgers: u32,
    pub run_index_refresh: bool,
}

pub struct PlannedWork {
    pub jobs: Vec<TxJob>,
    pub extend_targets: Vec<LedgerKey>,
}

/// Conservative cap on how many LedgerKeys to pack into a single
/// `ExtendFootprintTtl` op's read-only footprint. Soroban's per-tx footprint
/// limit is much higher (200+ entries) but a smaller bucket keeps the
/// per-op fee bounded and recoverable if a single tx is rejected.
const MAX_KEYS_PER_EXTEND_OP: usize = 60;

pub fn plan(input: &PlannerInput<'_>) -> Result<PlannedWork> {
    let snapshot = input.snapshot;
    let current_ledger = snapshot.current_ledger;
    let safety = input.safety_ledgers;

    let mut extend_targets: Vec<LedgerKey> = Vec::new();
    let mut counts = TierCounts::default();

    for row in &snapshot.persistent_entries {
        if needs_bump_decisive(&row.live_until_ledger, current_ledger, safety) {
            extend_targets.push(row.key.clone());
            counts.persistent += 1;
        }
    }
    for row in &snapshot.instance_entries {
        if needs_bump_decisive(&row.live_until_ledger, current_ledger, safety) {
            extend_targets.push(row.key.clone());
            counts.instance += 1;
        }
    }
    for row in &snapshot.wasm_code_entries {
        if needs_bump_decisive(&row.live_until_ledger, current_ledger, safety) {
            extend_targets.push(row.key.clone());
            counts.wasm += 1;
        }
    }

    let mut jobs = Vec::new();

    for chunk in extend_targets.chunks(MAX_KEYS_PER_EXTEND_OP) {
        jobs.push(extend_footprint_ttl(chunk, extend_to_ledgers())?);
    }

    if input.run_index_refresh && !snapshot.assets.is_empty() {
        for chunk in snapshot.assets.chunks(input.schedule.asset_chunk.max(1)) {
            let assets: Vec<[u8; 32]> = chunk.to_vec();
            jobs.push(update_indexes(input.controller_id, input.caller_strkey, &assets)?);
        }
    }

    debug!(
        target: "keeper.scheduler",
        n_jobs = jobs.len(),
        persistent_below_margin = counts.persistent,
        instance_below_margin = counts.instance,
        wasm_below_margin = counts.wasm,
        index_refresh_jobs = input.run_index_refresh as u32 * (snapshot.assets.len() as u32).div_ceil(input.schedule.asset_chunk.max(1) as u32),
        "plan built"
    );

    Ok(PlannedWork {
        jobs,
        extend_targets,
    })
}

#[derive(Default)]
struct TierCounts {
    persistent: usize,
    instance: usize,
    wasm: usize,
}

fn needs_bump_decisive(live_until: &Option<u32>, current_ledger: u32, safety: u32) -> bool {
    !matches!(
        needs_bump(*live_until, current_ledger, safety),
        BumpReason::Missing
    )
}

/// Ledgers-from-now to extend wasm-code entries to.
///
/// `ExtendFootprintTtlOp.extend_to` is a *count of ledgers from the
/// current ledger* (not an absolute sequence number), and Stellar caps it
/// at the network's `max_entry_ttl`. Protocol 26 testnet & mainnet sit at
/// 3,110,400 ledgers (~180 days), but ExtendFootprintTtl ops have a
/// stricter per-op cap. The well-known safe value used by SDF examples is
/// **535,679** (≈ 31 days), which is also the `MAX_LEDGERS_TO_EXTEND`
/// constant in stellar-core. Any value above that returns
/// `OpInner(ExtendFootprintTtl(Malformed))` at submission time.
///
/// We extend to 535,679 every tick; with a tick cadence well under that
/// window, entries stay perpetually fresh.
fn extend_to_ledgers() -> u32 {
    535_679
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::DiscoverySnapshot;
    use crate::stellar::client::LedgerEntryQuery;
    use stellar_xdr::curr::{
        ContractDataDurability, ContractId, Hash, LedgerKey, LedgerKeyContractData, ScAddress,
        ScVal,
    };

    fn fake_query(live_until: Option<u32>) -> LedgerEntryQuery {
        LedgerEntryQuery {
            key: LedgerKey::ContractData(LedgerKeyContractData {
                contract: ScAddress::Contract(ContractId(Hash([0u8; 32]))),
                key: ScVal::LedgerKeyContractInstance,
                durability: ContractDataDurability::Persistent,
            }),
            value: None,
            live_until_ledger: live_until,
        }
    }

    #[test]
    fn skips_entries_above_safety_margin() {
        let mut snap = DiscoverySnapshot {
            current_ledger: 100,
            ..Default::default()
        };
        // Plenty of headroom — should be skipped.
        snap.instance_entries.push(fake_query(Some(100 + 600_000)));

        let cfg = ScheduleConfig {
            ttl_tick_seconds: 0,
            index_tick_seconds: 0,
            ttl_safety_margin_days: 14,
            account_chunk: 50,
            asset_chunk: 20,
            max_txs_per_tick: 50,
            enable_index_refresh: false,
        };
        let plan = plan(&PlannerInput {
            snapshot: &snap,
            schedule: &cfg,
            controller_id: &[0u8; 32],
            caller_strkey: "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6",
            safety_ledgers: 14 * 17_280,
            run_index_refresh: false,
        })
        .unwrap();
        assert_eq!(plan.jobs.len(), 0);
    }

    #[test]
    fn batches_below_margin_into_chunked_extend_ops() {
        let mut snap = DiscoverySnapshot {
            current_ledger: 100,
            ..Default::default()
        };
        for _ in 0..125 {
            snap.persistent_entries.push(fake_query(Some(100 + 1_000)));
        }
        let cfg = ScheduleConfig {
            ttl_tick_seconds: 0,
            index_tick_seconds: 0,
            ttl_safety_margin_days: 14,
            account_chunk: 50,
            asset_chunk: 20,
            max_txs_per_tick: 50,
            enable_index_refresh: false,
        };
        let plan = plan(&PlannerInput {
            snapshot: &snap,
            schedule: &cfg,
            controller_id: &[0u8; 32],
            caller_strkey: "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6",
            safety_ledgers: 14 * 17_280,
            run_index_refresh: false,
        })
        .unwrap();
        // 125 keys at 60 per op → 3 jobs.
        assert_eq!(plan.jobs.len(), 3);
    }
}
