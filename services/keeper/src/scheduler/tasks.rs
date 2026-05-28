//! Translate a discovery snapshot into a list of `TxJob`s the submitter can
//! run. Pure function — no I/O.

use anyhow::Result;
use stellar_xdr::curr::{LedgerKey, ScVal};
use tracing::debug;

use crate::config::ScheduleConfig;
use crate::discovery::DiscoverySnapshot;
use crate::keys::contract_code_key;
use crate::policy::{needs_bump, BumpReason};
use crate::stellar::invoke::{
    keepalive_accounts, keepalive_pools, keepalive_shared_state, update_indexes,
};
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
    pub assets_needing_shared_bump: Vec<[u8; 32]>,
    pub assets_needing_pool_bump: Vec<[u8; 32]>,
    pub accounts_needing_bump: Vec<u64>,
    pub wasm_extend_targets: Vec<LedgerKey>,
}

pub fn plan(input: &PlannerInput<'_>) -> Result<PlannedWork> {
    let snapshot = input.snapshot;
    let current_ledger = snapshot.current_ledger;
    let safety = input.safety_ledgers;

    let mut assets_needing_shared_bump = Vec::new();
    let mut assets_needing_pool_bump = Vec::new();
    let mut accounts_needing_bump = Vec::new();

    for row in &snapshot.persistent_entries {
        let reason = needs_bump(row.live_until_ledger, current_ledger, safety);
        if matches!(reason, BumpReason::Missing) {
            continue;
        }
        // Pull asset / id out of the LedgerKey for routing.
        match classify_persistent_key(&row.key) {
            Some(PersistentClass::Market(asset)) => {
                push_unique(&mut assets_needing_shared_bump, asset);
                push_unique(&mut assets_needing_pool_bump, asset);
            }
            Some(PersistentClass::IsolatedDebt(asset)) => {
                push_unique(&mut assets_needing_shared_bump, asset);
            }
            Some(PersistentClass::EModeCategory(_)) => {
                // Covered transitively by keepalive_shared_state for the
                // assets referencing the category. No-op here.
            }
            Some(PersistentClass::AccountTriplet(id)) => {
                push_unique(&mut accounts_needing_bump, id);
            }
            Some(PersistentClass::PoolsList) | None => {
                // PoolsList rides along with keepalive_shared_state; missing
                // routing for a key we don't recognize is logged but skipped.
            }
        }
    }

    let mut wasm_extend_targets = Vec::new();
    for row in &snapshot.wasm_code_entries {
        let reason = needs_bump(row.live_until_ledger, current_ledger, safety);
        if matches!(reason, BumpReason::Missing) {
            continue;
        }
        if let LedgerKey::ContractCode(code) = &row.key {
            wasm_extend_targets.push(LedgerKey::ContractCode(code.clone()));
        }
    }
    // Pool template hash always carried, even if discovery row was absent
    // (defensive — a brand-new deploy may not have the entry yet).
    let _ = contract_code_key; // imported for completeness

    let mut jobs = Vec::new();

    // -- update_indexes runs on a different cadence; gate by caller flag --
    if input.run_index_refresh && !snapshot.assets.is_empty() {
        for chunk in snapshot.assets.chunks(input.schedule.asset_chunk.max(1)) {
            let assets: Vec<[u8; 32]> = chunk.to_vec();
            jobs.push(update_indexes(input.controller_id, input.caller_strkey, &assets)?);
        }
    }

    // -- keepalive_shared_state batches --
    for chunk in assets_needing_shared_bump.chunks(input.schedule.asset_chunk.max(1)) {
        let assets: Vec<[u8; 32]> = chunk.to_vec();
        jobs.push(keepalive_shared_state(
            input.controller_id,
            input.caller_strkey,
            &assets,
        )?);
    }

    // -- keepalive_pools batches --
    for chunk in assets_needing_pool_bump.chunks(input.schedule.asset_chunk.max(1)) {
        let assets: Vec<[u8; 32]> = chunk.to_vec();
        jobs.push(keepalive_pools(input.controller_id, input.caller_strkey, &assets)?);
    }

    // -- keepalive_accounts batches --
    for chunk in accounts_needing_bump.chunks(input.schedule.account_chunk.max(1)) {
        let ids: Vec<u64> = chunk.to_vec();
        jobs.push(keepalive_accounts(input.controller_id, input.caller_strkey, &ids)?);
    }

    // -- ExtendFootprintTtl batch (single tx covers all wasm hashes) --
    if !wasm_extend_targets.is_empty() {
        let extend_to = bump_target_ledger(current_ledger);
        jobs.push(extend_footprint_ttl(&wasm_extend_targets, extend_to)?);
    }

    debug!(
        target: "keeper.scheduler",
        n_jobs = jobs.len(),
        n_shared_assets = assets_needing_shared_bump.len(),
        n_pool_assets = assets_needing_pool_bump.len(),
        n_accounts = accounts_needing_bump.len(),
        n_wasm = wasm_extend_targets.len(),
        "plan built"
    );

    Ok(PlannedWork {
        jobs,
        assets_needing_shared_bump,
        assets_needing_pool_bump,
        accounts_needing_bump,
        wasm_extend_targets,
    })
}

fn bump_target_ledger(current_ledger: u32) -> u32 {
    // Match the contracts' shared-tier bump: 180 days.
    const ONE_DAY_LEDGERS: u32 = 17_280;
    current_ledger.saturating_add(180 * ONE_DAY_LEDGERS)
}

#[derive(Debug)]
enum PersistentClass {
    Market([u8; 32]),
    IsolatedDebt([u8; 32]),
    EModeCategory(#[allow(dead_code)] u32),
    AccountTriplet(u64),
    PoolsList,
}

fn classify_persistent_key(key: &LedgerKey) -> Option<PersistentClass> {
    let LedgerKey::ContractData(cd) = key else {
        return None;
    };
    let ScVal::Vec(Some(inner)) = &cd.key else { return None };
    if inner.0.is_empty() {
        return None;
    }
    let ScVal::Symbol(name) = &inner.0[0] else { return None };
    let name = name.0.to_utf8_string_lossy();
    use stellar_xdr::curr::{ContractId, Hash, ScAddress};
    match (name.as_str(), inner.0.get(1)) {
        ("PoolsList", _) => Some(PersistentClass::PoolsList),
        ("Market", Some(ScVal::Address(ScAddress::Contract(ContractId(Hash(b)))))) => {
            Some(PersistentClass::Market(*b))
        }
        ("IsolatedDebt", Some(ScVal::Address(ScAddress::Contract(ContractId(Hash(b)))))) => {
            Some(PersistentClass::IsolatedDebt(*b))
        }
        ("EModeCategory", Some(ScVal::U32(id))) => Some(PersistentClass::EModeCategory(*id)),
        ("AccountMeta", Some(ScVal::U64(id)))
        | ("SupplyPositions", Some(ScVal::U64(id)))
        | ("BorrowPositions", Some(ScVal::U64(id))) => Some(PersistentClass::AccountTriplet(*id)),
        _ => None,
    }
}

fn push_unique<T: PartialEq>(buf: &mut Vec<T>, item: T) {
    if !buf.iter().any(|existing| existing == &item) {
        buf.push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::curr::{
        ContractDataDurability, ContractId, Hash, LedgerKey, LedgerKeyContractData, ScAddress,
        ScSymbol, ScVal, ScVec, StringM, VecM,
    };

    fn make_key(variant: &str, arg: Option<ScVal>) -> LedgerKey {
        let mut elems = vec![ScVal::Symbol(ScSymbol(
            StringM::<32>::try_from(variant).unwrap(),
        ))];
        if let Some(v) = arg {
            elems.push(v);
        }
        let vec_m: VecM<ScVal> = elems.try_into().unwrap();
        LedgerKey::ContractData(LedgerKeyContractData {
            contract: ScAddress::Contract(ContractId(Hash([0u8; 32]))),
            key: ScVal::Vec(Some(ScVec(vec_m))),
            durability: ContractDataDurability::Persistent,
        })
    }

    #[test]
    fn classifies_market_key() {
        let k = make_key(
            "Market",
            Some(ScVal::Address(ScAddress::Contract(ContractId(Hash([7u8; 32]))))),
        );
        assert!(matches!(
            classify_persistent_key(&k),
            Some(PersistentClass::Market(b)) if b == [7u8; 32]
        ));
    }

    #[test]
    fn classifies_account_triplet() {
        let k = make_key("AccountMeta", Some(ScVal::U64(99)));
        assert!(matches!(
            classify_persistent_key(&k),
            Some(PersistentClass::AccountTriplet(99))
        ));
    }
}
