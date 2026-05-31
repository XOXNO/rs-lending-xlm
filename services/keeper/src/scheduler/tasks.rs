//! Translate a discovery snapshot into the `TxJob`s the submitter ships.
//! Pure functions — no I/O.
//!
//! TTL bumping is permissionless: [`plan_extends`] emits `ExtendFootprintTtl`
//! ops covering every entry whose `live_until` is inside the safety margin
//! (persistent storage, contract instances, wasm code), requiring no on-chain
//! role. [`plan_index_refresh`] builds the keeper's only role-gated work —
//! `update_indexes`, which advances pool interest accrual and needs the KEEPER
//! role on the signer.

use anyhow::Result;
use stellar_xdr::curr::LedgerKey;
use tracing::debug;

use crate::discovery::DiscoverySnapshot;
use crate::policy::needs_bump;
use crate::stellar::client::LedgerEntryQuery;
use crate::stellar::invoke::update_indexes;
use crate::stellar::ttl::{extend_footprint_ttl, MAX_LEDGERS_TO_EXTEND};
use crate::stellar::TxJob;

/// Conservative cap on how many LedgerKeys to pack into a single
/// `ExtendFootprintTtl` op's read-only footprint. Soroban's per-tx footprint
/// limit is higher (200+ entries), but a smaller bucket bounds the per-op fee
/// and keeps a rejected tx cheap to retry.
const MAX_KEYS_PER_EXTEND_OP: usize = 60;

/// Build the permissionless TTL-extend jobs covering every entry in `snapshot`
/// whose `live_until` is inside the safety margin, chunked to bound per-op fee.
pub fn plan_extends(snapshot: &DiscoverySnapshot, safety_ledgers: u32) -> Result<Vec<TxJob>> {
    let current_ledger = snapshot.current_ledger;
    let mut targets: Vec<LedgerKey> = Vec::with_capacity(
        snapshot.persistent_entries.len()
            + snapshot.instance_entries.len()
            + snapshot.wasm_code_entries.len(),
    );

    let persistent =
        collect_below_margin(&snapshot.persistent_entries, current_ledger, safety_ledgers, &mut targets);
    let instance =
        collect_below_margin(&snapshot.instance_entries, current_ledger, safety_ledgers, &mut targets);
    let wasm =
        collect_below_margin(&snapshot.wasm_code_entries, current_ledger, safety_ledgers, &mut targets);

    let mut jobs = Vec::with_capacity(targets.len().div_ceil(MAX_KEYS_PER_EXTEND_OP));
    for chunk in targets.chunks(MAX_KEYS_PER_EXTEND_OP) {
        jobs.push(extend_footprint_ttl(chunk, MAX_LEDGERS_TO_EXTEND)?);
    }

    debug!(
        target: "keeper.scheduler",
        n_jobs = jobs.len(),
        persistent_below_margin = persistent,
        instance_below_margin = instance,
        wasm_below_margin = wasm,
        "extend plan built"
    );
    Ok(jobs)
}

/// Build the role-gated `update_indexes(assets)` jobs, chunked by `asset_chunk`.
pub fn plan_index_refresh(
    controller_id: &[u8; 32],
    caller_strkey: &str,
    assets: &[[u8; 32]],
    asset_chunk: usize,
) -> Result<Vec<TxJob>> {
    let mut jobs = Vec::new();
    for chunk in assets.chunks(asset_chunk.max(1)) {
        jobs.push(update_indexes(controller_id, caller_strkey, chunk)?);
    }
    Ok(jobs)
}

/// Push the keys of every entry inside the safety margin onto `out`, returning
/// how many were added.
fn collect_below_margin(
    entries: &[LedgerEntryQuery],
    current_ledger: u32,
    safety_ledgers: u32,
    out: &mut Vec<LedgerKey>,
) -> usize {
    let mut added = 0;
    for row in entries {
        if needs_bump(row.live_until_ledger, current_ledger, safety_ledgers) {
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
    use stellar_xdr::curr::{
        ContractDataDurability, ContractId, Hash, LedgerKey, LedgerKeyContractData, ScAddress,
        ScVal,
    };

    const TEST_PUBKEY: &str = "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6";

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
            snap.persistent_entries.push(fake_query(Some(100 + 1_000)));
        }
        let jobs = plan_extends(&snap, 14 * LEDGERS_PER_DAY).unwrap();
        // 125 keys at 60 per op → 3 jobs.
        assert_eq!(jobs.len(), 3);
    }

    #[test]
    fn index_refresh_chunks_assets_by_asset_chunk() {
        let assets: Vec<[u8; 32]> = (0..45u8).map(|i| [i; 32]).collect();
        let jobs = plan_index_refresh(&[0u8; 32], TEST_PUBKEY, &assets, 20).unwrap();
        // 45 assets at 20 per op → 3 jobs.
        assert_eq!(jobs.len(), 3);
    }
}
