//! Converts discovery snapshots into transaction jobs.

use anyhow::Result;
use stellar_xdr::curr::LedgerKey;
use tracing::debug;

use crate::discovery::DiscoverySnapshot;
use crate::keys::HubAssetKey;
use crate::policy::{classify, Decision};
use crate::stellar::client::LedgerEntryQuery;
use crate::stellar::invoke::update_indexes;
use crate::stellar::restore::restore_footprint;
use crate::stellar::ttl::{extend_footprint_ttl, MAX_LEDGERS_TO_EXTEND};
use crate::stellar::TxJob;

/// Max ledger keys per footprint op.
const MAX_KEYS_PER_EXTEND_OP: usize = 60;

/// TTL-extend jobs for entries inside the safety margin.
pub fn plan_extends(snapshot: &DiscoverySnapshot, safety_ledgers: u32) -> Result<Vec<TxJob>> {
    plan_extends_with_chunk(snapshot, safety_ledgers, MAX_KEYS_PER_EXTEND_OP)
}

/// Like `plan_extends` with an explicit per-tx key cap. Smaller chunks keep
/// month-scale rent (incl. large wasm code) under the classic envelope u32 fee cap.
pub fn plan_extends_with_chunk(
    snapshot: &DiscoverySnapshot,
    safety_ledgers: u32,
    chunk: usize,
) -> Result<Vec<TxJob>> {
    plan_with_chunk(snapshot, safety_ledgers, Decision::Extend, chunk, |chunk| {
        extend_footprint_ttl(chunk, MAX_LEDGERS_TO_EXTEND)
    })
}

/// Restore jobs for archived entries with data still present.
pub fn plan_restores(snapshot: &DiscoverySnapshot, safety_ledgers: u32) -> Result<Vec<TxJob>> {
    plan(
        snapshot,
        safety_ledgers,
        Decision::Restore,
        restore_footprint,
    )
}

fn plan(
    snapshot: &DiscoverySnapshot,
    safety_ledgers: u32,
    want: Decision,
    build: impl Fn(&[LedgerKey]) -> Result<TxJob>,
) -> Result<Vec<TxJob>> {
    plan_with_chunk(snapshot, safety_ledgers, want, MAX_KEYS_PER_EXTEND_OP, build)
}

fn plan_with_chunk(
    snapshot: &DiscoverySnapshot,
    safety_ledgers: u32,
    want: Decision,
    chunk: usize,
    build: impl Fn(&[LedgerKey]) -> Result<TxJob>,
) -> Result<Vec<TxJob>> {
    let current_ledger = snapshot.current_ledger;
    let mut targets: Vec<LedgerKey> = Vec::new();

    let persistent = collect_matching(
        &snapshot.persistent_entries,
        current_ledger,
        safety_ledgers,
        want,
        &mut targets,
    );
    let instance = collect_matching(
        &snapshot.instance_entries,
        current_ledger,
        safety_ledgers,
        want,
        &mut targets,
    );
    let wasm = collect_matching(
        &snapshot.wasm_code_entries,
        current_ledger,
        safety_ledgers,
        want,
        &mut targets,
    );

    let mut jobs = Vec::with_capacity(targets.len().div_ceil(chunk));
    for chunk in targets.chunks(chunk) {
        jobs.push(build(chunk)?);
    }

    debug!(
        target: "keeper.scheduler",
        plan = ?want,
        n_jobs = jobs.len(),
        persistent_matched = persistent,
        instance_matched = instance,
        wasm_matched = wasm,
        "plan built"
    );
    Ok(jobs)
}

/// Read-write keys from restore jobs.
pub fn restored_keys(jobs: &[TxJob]) -> Vec<LedgerKey> {
    jobs.iter()
        .filter(|j| matches!(j.kind, crate::stellar::tx::TxKind::RestoreFootprint))
        .filter_map(|j| j.initial_soroban_data.as_ref())
        .flat_map(|data| data.resources.footprint.read_write.iter().cloned())
        .collect()
}

/// Extend jobs for an explicit key set.
pub fn plan_extends_for_keys(keys: &[LedgerKey]) -> Result<Vec<TxJob>> {
    let mut jobs = Vec::with_capacity(keys.len().div_ceil(MAX_KEYS_PER_EXTEND_OP));
    for chunk in keys.chunks(MAX_KEYS_PER_EXTEND_OP) {
        jobs.push(extend_footprint_ttl(chunk, MAX_LEDGERS_TO_EXTEND)?);
    }
    Ok(jobs)
}

/// `update_indexes(hub_assets)` jobs.
pub fn plan_index_refresh(
    controller_id: &[u8; 32],
    caller_strkey: &str,
    assets: &[HubAssetKey],
    asset_chunk: usize,
) -> Result<Vec<TxJob>> {
    let mut jobs = Vec::new();
    for chunk in assets.chunks(asset_chunk.max(1)) {
        jobs.push(update_indexes(controller_id, caller_strkey, chunk)?);
    }
    Ok(jobs)
}

fn collect_matching(
    entries: &[LedgerEntryQuery],
    current_ledger: u32,
    safety_ledgers: u32,
    want: Decision,
    out: &mut Vec<LedgerKey>,
) -> usize {
    let mut added = 0;
    for row in entries {
        let decision = classify(
            row.live_until_ledger,
            row.value.is_some(),
            current_ledger,
            safety_ledgers,
        );
        if decision == want {
            out.push(row.key.clone());
            added += 1;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LEDGERS_PER_DAY;
    use crate::discovery::DiscoverySnapshot;
    use crate::stellar::tx::TxKind;
    use stellar_xdr::curr::{
        ContractDataDurability, ContractDataEntry, ContractId, ExtensionPoint, Hash,
        LedgerEntryData, LedgerKey, LedgerKeyContractData, ScAddress, ScVal,
    };

    const TEST_PUBKEY: &str = "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6";

    fn fake_key() -> LedgerKey {
        LedgerKey::ContractData(LedgerKeyContractData {
            contract: ScAddress::Contract(ContractId(Hash([0u8; 32]))),
            key: ScVal::LedgerKeyContractInstance,
            durability: ContractDataDurability::Persistent,
        })
    }

    fn present(live_until: Option<u32>) -> LedgerEntryQuery {
        LedgerEntryQuery {
            key: fake_key(),
            value: Some(LedgerEntryData::ContractData(ContractDataEntry {
                ext: ExtensionPoint::V0,
                contract: ScAddress::Contract(ContractId(Hash([0u8; 32]))),
                key: ScVal::LedgerKeyContractInstance,
                durability: ContractDataDurability::Persistent,
                val: ScVal::Void,
            })),
            live_until_ledger: live_until,
        }
    }

    fn absent(live_until: Option<u32>) -> LedgerEntryQuery {
        LedgerEntryQuery {
            key: fake_key(),
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
        snap.instance_entries.push(present(Some(100 + 600_000)));
        let jobs = plan_extends(&snap, 14 * LEDGERS_PER_DAY).unwrap();
        assert_eq!(jobs.len(), 0);
    }

    #[test]
    fn batches_below_margin_into_chunked_extend_ops() {
        let mut snap = DiscoverySnapshot {
            current_ledger: 100,
            ..Default::default()
        };
        for _ in 0..125 {
            snap.persistent_entries.push(present(Some(100 + 1_000)));
        }
        let jobs = plan_extends(&snap, 14 * LEDGERS_PER_DAY).unwrap();
        assert_eq!(jobs.len(), 3); // 125 keys @ 60/op
    }

    #[test]
    fn extend_skips_archived_and_absent_entries() {
        let mut snap = DiscoverySnapshot {
            current_ledger: 100,
            ..Default::default()
        };
        snap.persistent_entries.push(present(Some(50))); // archived
        snap.persistent_entries.push(absent(Some(50))); // evicted
        let jobs = plan_extends(&snap, 14 * LEDGERS_PER_DAY).unwrap();
        assert_eq!(jobs.len(), 0);
    }

    #[test]
    fn restore_batches_archived_present_entries_into_chunks() {
        let mut snap = DiscoverySnapshot {
            current_ledger: 1_000,
            ..Default::default()
        };
        for _ in 0..125 {
            snap.persistent_entries.push(present(Some(0)));
        }
        let jobs = plan_restores(&snap, 14 * LEDGERS_PER_DAY).unwrap();
        assert_eq!(jobs.len(), 3);
        assert!(jobs.iter().all(|j| j.kind == TxKind::RestoreFootprint));
    }

    #[test]
    fn restore_skips_live_and_absent_entries() {
        let mut snap = DiscoverySnapshot {
            current_ledger: 1_000,
            ..Default::default()
        };
        snap.persistent_entries.push(present(Some(1_010))); // live, in margin
        snap.persistent_entries.push(absent(Some(0)));
        let jobs = plan_restores(&snap, 14 * LEDGERS_PER_DAY).unwrap();
        assert_eq!(jobs.len(), 0);
    }

    #[test]
    fn restored_keys_extracts_restore_read_write_targets() {
        let mut snap = DiscoverySnapshot {
            current_ledger: 1_000,
            ..Default::default()
        };
        snap.persistent_entries.push(present(Some(0)));
        snap.persistent_entries.push(present(Some(0)));
        let restores = plan_restores(&snap, 14 * LEDGERS_PER_DAY).unwrap();
        assert_eq!(restored_keys(&restores).len(), 2);
        let extends = plan_extends_for_keys(&restored_keys(&restores)).unwrap();
        assert_eq!(extends.len(), 1);
    }

    #[test]
    fn index_refresh_chunks_assets_by_asset_chunk() {
        let assets: Vec<HubAssetKey> = (0..45u8)
            .map(|i| HubAssetKey {
                hub_id: 1,
                asset: [i; 32],
            })
            .collect();
        let jobs = plan_index_refresh(&[0u8; 32], TEST_PUBKEY, &assets, 20).unwrap();
        assert_eq!(jobs.len(), 3); // 45 assets @ 20/op
    }
}
