//! Read controller + pool storage, build the lists of work the scheduler
//! turns into transactions.

use anyhow::{anyhow, Context, Result};
use stellar_xdr::curr::{
    ContractExecutable, ContractId, Hash, LedgerEntryData, LedgerKey, ScAddress,
    ScContractInstance, ScMapEntry, ScSymbol, ScVal, StringM,
};
use tracing::{debug, info, warn};

use crate::config::ContractsConfig;
use crate::keys::{
    contract_code_key, contract_instance_key, AccessControlPersistentKey, ControllerInstanceKey,
    ControllerPersistentKey,
};
use crate::stellar::client::{
    contract_id_from_strkey, hash32_from_hex, muxed_account_from_strkey, LedgerEntryQuery, RpcClient,
};

/// Contract identities parsed once at startup from `ContractsConfig`. Parsing
/// strkeys and hex is a boundary concern; once resolved, the keeper works in
/// raw 32-byte ids and never re-parses them per tick.
#[derive(Debug, Clone, Copy)]
pub struct ContractIds {
    pub controller: [u8; 32],
    pub pool_wasm_hash: [u8; 32],
    pub flash_receiver: [u8; 32],
}

impl ContractIds {
    pub fn resolve(contracts: &ContractsConfig) -> Result<Self> {
        Ok(Self {
            controller: contract_id_from_strkey(&contracts.controller)?,
            pool_wasm_hash: hash32_from_hex(&contracts.pool_wasm_hash)?,
            flash_receiver: contract_id_from_strkey(&contracts.flash_loan_receiver)?,
        })
    }
}

/// One snapshot of "what needs bumping" assembled by a single tick. Holds only
/// what the planner and metrics consume; intermediate ids resolved while
/// reading stay local to [`snapshot`].
#[derive(Debug, Default)]
pub struct DiscoverySnapshot {
    pub current_ledger: u32,
    pub assets: Vec<[u8; 32]>,
    /// Persistent ledger entries we may want to bump (PoolsList, per-asset
    /// Market + IsolatedDebt, per-category EModeCategory). Each row carries its
    /// `live_until` and the decoded key. Per-user triplets are excluded by
    /// design — users auto-bump their own keys.
    pub persistent_entries: Vec<LedgerEntryQuery>,
    /// Contract-instance entries (controller + pools + flash receiver). Bumping
    /// the instance entry covers all instance-tier storage, including the
    /// oracle `Aggregator` and the rest of the controller's instance keys.
    pub instance_entries: Vec<LedgerEntryQuery>,
    /// Wasm code entries (controller + pool template + flash receiver). These
    /// are the only entries a contract cannot self-extend.
    pub wasm_code_entries: Vec<LedgerEntryQuery>,
    /// Account-id ceiling read from controller instance storage, surfaced as
    /// the `keeper_account_nonce` metric. The keeper does not bump per-user
    /// keys, so this is observability only — a moving value signals that the
    /// controller is in active use.
    pub account_nonce: u64,
}

pub async fn snapshot(
    client: &RpcClient,
    ids: &ContractIds,
    asset_chunk: usize,
) -> Result<DiscoverySnapshot> {
    let chunk_size = asset_chunk.max(1);
    let controller_id = ids.controller;

    let current_ledger = client.latest_ledger().await?;
    info!(target: "keeper.discovery", current_ledger, "tick start");

    // -- Controller instance: wasm hash + AccountNonce + e-mode ceiling --
    let instance = client.get_contract_instance(&controller_id).await?;
    let controller_wasm_hash = wasm_hash_from_executable(&instance.executable);
    let account_nonce =
        lookup_scalar(&instance, ControllerInstanceKey::AccountNonce, scval_u64)?.unwrap_or(0);
    let last_emode_category_id =
        lookup_scalar(&instance, ControllerInstanceKey::LastEModeCategoryId, scval_u32)?
            .unwrap_or(0);
    debug!(
        target: "keeper.discovery",
        account_nonce,
        last_emode_category_id,
        "instance read"
    );

    // -- Pool list (persistent) --
    let pool_list_key = ControllerPersistentKey::PoolsList.to_ledger_key(&controller_id)?;
    let mut persistent_entries = client.get_ledger_entries(&[pool_list_key]).await?;
    let assets = extract_pools_list(&persistent_entries).unwrap_or_default();

    // -- Per-asset persistent state: Market + IsolatedDebt --
    let mut pool_contract_ids: Vec<[u8; 32]> = Vec::with_capacity(assets.len());
    for chunk in assets.chunks(chunk_size) {
        let mut keys = Vec::with_capacity(chunk.len() * 2);
        for asset in chunk {
            keys.push(ControllerPersistentKey::Market(*asset).to_ledger_key(&controller_id)?);
            keys.push(ControllerPersistentKey::IsolatedDebt(*asset).to_ledger_key(&controller_id)?);
        }
        for row in client.get_ledger_entries(&keys).await? {
            // Each Market entry names the pool contract it backs; harvest those
            // ids for instance bumping, then keep the Market row for TTL.
            if let Some(pool_id) = extract_pool_address_from_market(&row) {
                pool_contract_ids.push(pool_id);
            }
            persistent_entries.push(row);
        }
    }

    // -- E-mode category sweep (1..=ceiling) --
    if last_emode_category_id > 0 {
        for chunk in (1..=last_emode_category_id).collect::<Vec<_>>().chunks(chunk_size) {
            let keys = chunk
                .iter()
                .map(|id| ControllerPersistentKey::EModeCategory(*id).to_ledger_key(&controller_id))
                .collect::<Result<Vec<_>>>()?;
            persistent_entries.extend(client.get_ledger_entries(&keys).await?);
        }
    }

    // -- Access-control role keys (ExistingRoles, per-role RoleAccountsCount,
    //    per-(role,index) RoleAccounts, per-(holder,role) HasRole) --
    persistent_entries.extend(discover_role_keys(client, &controller_id, chunk_size).await?);

    // -- Instance entries (controller + each pool + flash receiver) --
    let mut instance_keys = Vec::with_capacity(pool_contract_ids.len() + 2);
    instance_keys.push(contract_instance_key(&controller_id));
    for pool_id in &pool_contract_ids {
        instance_keys.push(contract_instance_key(pool_id));
    }
    instance_keys.push(contract_instance_key(&ids.flash_receiver));
    let instance_entries = client.get_ledger_entries(&instance_keys).await?;

    // -- Wasm code entries (pool template + controller + flash receiver) --
    let mut wasm_keys: Vec<LedgerKey> = vec![contract_code_key(&ids.pool_wasm_hash)];
    if let Some(ctrl_hash) = controller_wasm_hash {
        wasm_keys.push(contract_code_key(&ctrl_hash));
    } else {
        warn!(target: "keeper.discovery", "controller wasm hash unresolved — pool template extend only");
    }
    // The flash-receiver wasm hash lives in the instance entry we just read.
    if let Some(flash_hash) = instance_entries.last().and_then(wasm_hash_from_instance_row) {
        wasm_keys.push(contract_code_key(&flash_hash));
    }
    let wasm_code_entries = client.get_ledger_entries(&wasm_keys).await?;

    Ok(DiscoverySnapshot {
        current_ledger,
        assets,
        persistent_entries,
        instance_entries,
        wasm_code_entries,
        account_nonce,
    })
}

/// Operational roles assumed when `ExistingRoles` itself cannot be read.
const DEFAULT_ROLES: [&str; 3] = ["KEEPER", "REVENUE", "ORACLE"];

/// Read the controller's access-control persistent keys so the keeper keeps
/// them alive. They self-extend only when a role-gated call reads them, so an
/// idle protocol silently lets them archive. Archived-but-present entries are
/// still decodable (the RPC returns their data), so enumeration works even when
/// the role bookkeeping has already lapsed.
async fn discover_role_keys(
    client: &RpcClient,
    controller_id: &[u8; 32],
    chunk_size: usize,
) -> Result<Vec<LedgerEntryQuery>> {
    let mut rows: Vec<LedgerEntryQuery> = Vec::new();

    // ExistingRoles → the set of role names to enumerate.
    let existing_key = AccessControlPersistentKey::ExistingRoles.to_ledger_key(controller_id)?;
    let existing_rows = client.get_ledger_entries(&[existing_key]).await?;
    let roles = extract_existing_roles(&existing_rows)
        .unwrap_or_else(|| DEFAULT_ROLES.iter().map(|s| s.to_string()).collect());
    rows.extend(existing_rows);

    // Per-role RoleAccountsCount.
    let count_keys = roles
        .iter()
        .map(|r| AccessControlPersistentKey::RoleAccountsCount(r.clone()).to_ledger_key(controller_id))
        .collect::<Result<Vec<_>>>()?;
    let count_rows = client.get_ledger_entries(&count_keys).await?;
    let counts: Vec<(String, u32)> = roles
        .iter()
        .cloned()
        .zip(count_rows.iter().map(|r| extract_u32(r).unwrap_or(0)))
        .collect();
    rows.extend(count_rows);

    // Per-(role, index) RoleAccounts; the value names the holder address.
    let mut ra_keys = Vec::new();
    let mut ra_meta: Vec<String> = Vec::new();
    for (role, count) in &counts {
        for index in 0..*count {
            ra_keys.push(
                AccessControlPersistentKey::RoleAccounts(role.clone(), index)
                    .to_ledger_key(controller_id)?,
            );
            ra_meta.push(role.clone());
        }
    }
    let mut ra_rows = Vec::with_capacity(ra_keys.len());
    for chunk in ra_keys.chunks(chunk_size.max(1)) {
        ra_rows.extend(client.get_ledger_entries(chunk).await?);
    }

    // Per-(holder, role) HasRole, built from the holders just read.
    let mut hr_keys = Vec::new();
    for (role, row) in ra_meta.iter().zip(ra_rows.iter()) {
        if let Some(addr) = extract_address(row) {
            hr_keys.push(
                AccessControlPersistentKey::HasRole(addr, role.clone())
                    .to_ledger_key(controller_id)?,
            );
        }
    }
    rows.extend(ra_rows);
    for chunk in hr_keys.chunks(chunk_size.max(1)) {
        rows.extend(client.get_ledger_entries(chunk).await?);
    }

    debug!(
        target: "keeper.discovery",
        roles = roles.len(),
        role_entries = rows.len(),
        "role keys discovered"
    );
    Ok(rows)
}

/// Decode `ExistingRoles` (`Vec<Symbol>`) into role-name strings.
fn extract_existing_roles(rows: &[LedgerEntryQuery]) -> Option<Vec<String>> {
    let LedgerEntryData::ContractData(cd) = rows.first()?.value.as_ref()? else {
        return None;
    };
    let ScVal::Vec(Some(vec)) = &cd.val else {
        return None;
    };
    let out: Vec<String> = vec
        .0
        .iter()
        .filter_map(|v| match v {
            ScVal::Symbol(ScSymbol(s)) => Some(s.to_utf8_string_lossy()),
            _ => None,
        })
        .collect();
    (!out.is_empty()).then_some(out)
}

fn extract_u32(row: &LedgerEntryQuery) -> Option<u32> {
    let LedgerEntryData::ContractData(cd) = row.value.as_ref()? else {
        return None;
    };
    match cd.val {
        ScVal::U32(n) => Some(n),
        _ => None,
    }
}

fn extract_address(row: &LedgerEntryQuery) -> Option<ScAddress> {
    let LedgerEntryData::ContractData(cd) = row.value.as_ref()? else {
        return None;
    };
    match &cd.val {
        ScVal::Address(addr) => Some(addr.clone()),
        _ => None,
    }
}

/// Walk a Market entry's `ScVal::Map` and pull out the `pool_address` field.
fn extract_pool_address_from_market(row: &LedgerEntryQuery) -> Option<[u8; 32]> {
    let LedgerEntryData::ContractData(cd) = row.value.as_ref()? else {
        return None;
    };
    let ScVal::Map(Some(map)) = &cd.val else {
        return None;
    };
    for ScMapEntry { key, val } in map.0.iter() {
        let ScVal::Symbol(ScSymbol(sym)) = key else {
            continue;
        };
        if sym.to_utf8_string_lossy() == "pool_address" {
            if let ScVal::Address(ScAddress::Contract(ContractId(Hash(bytes)))) = val {
                return Some(*bytes);
            }
        }
    }
    None
}

fn wasm_hash_from_executable(executable: &ContractExecutable) -> Option<[u8; 32]> {
    match executable {
        ContractExecutable::Wasm(Hash(bytes)) => Some(*bytes),
        ContractExecutable::StellarAsset => None,
    }
}

fn wasm_hash_from_instance_row(row: &LedgerEntryQuery) -> Option<[u8; 32]> {
    let LedgerEntryData::ContractData(cd) = row.value.as_ref()? else {
        return None;
    };
    let ScVal::ContractInstance(inst) = &cd.val else {
        return None;
    };
    wasm_hash_from_executable(&inst.executable)
}

fn scval_u64(val: &ScVal) -> Option<u64> {
    match val {
        ScVal::U64(v) => Some(*v),
        _ => None,
    }
}

fn scval_u32(val: &ScVal) -> Option<u32> {
    match val {
        ScVal::U32(v) => Some(*v),
        _ => None,
    }
}

/// Find an instance-storage scalar by key and decode it with `extract`.
fn lookup_scalar<T>(
    instance: &ScContractInstance,
    key: ControllerInstanceKey,
    extract: impl Fn(&ScVal) -> Option<T>,
) -> Result<Option<T>> {
    let needle = needle_for(key)?;
    let Some(storage) = &instance.storage else {
        return Ok(None);
    };
    for ScMapEntry { key, val } in storage.0.iter() {
        if key == &needle {
            return Ok(extract(val));
        }
    }
    Ok(None)
}

fn needle_for(key: ControllerInstanceKey) -> Result<ScVal> {
    let symbol = ScSymbol(
        StringM::<32>::try_from(key.variant_name()).map_err(|_| anyhow!("symbol too long"))?,
    );
    Ok(ScVal::Vec(Some(stellar_xdr::curr::ScVec(
        vec![ScVal::Symbol(symbol)]
            .try_into()
            .map_err(|_| anyhow!("vec convert"))?,
    ))))
}

fn extract_pools_list(rows: &[LedgerEntryQuery]) -> Option<Vec<[u8; 32]>> {
    let row = rows.first()?;
    let LedgerEntryData::ContractData(cd) = row.value.as_ref()? else {
        return None;
    };
    let ScVal::Vec(Some(vec)) = &cd.val else {
        return None;
    };
    let mut out = Vec::with_capacity(vec.0.len());
    for v in vec.0.iter() {
        if let ScVal::Address(ScAddress::Contract(ContractId(Hash(bytes)))) = v {
            out.push(*bytes);
        }
    }
    Some(out)
}

/// Verify our ControllerKey encoding by reading `PoolsList` from the live
/// controller. Returns the decoded asset list (which may be empty for a
/// fresh deployment — emptiness is not an error).
pub async fn self_check(client: &RpcClient, controller_strkey: &str) -> Result<Vec<[u8; 32]>> {
    let controller_id = contract_id_from_strkey(controller_strkey)?;
    let key = ControllerPersistentKey::PoolsList.to_ledger_key(&controller_id)?;
    let rows = client.get_ledger_entries(std::slice::from_ref(&key)).await?;
    let row = rows
        .first()
        .ok_or_else(|| anyhow!("get_ledger_entries returned no row for PoolsList"))?;
    if row.value.is_none() {
        // A missing entry means either a fresh controller (empty list) or a
        // broken encoding. soroban-sdk still stores an empty `Vec`, so absence
        // is suspicious but not definitive.
        warn!(target: "keeper.discovery", "PoolsList missing from ledger — controller may be fresh");
        return Ok(Vec::new());
    }
    Ok(extract_pools_list(&rows).unwrap_or_default())
}

/// Boot-time auth gate for the optional index-refresh loop: simulate
/// `update_indexes(caller, empty_vec)` and refuse to start unless simulation
/// succeeds, which confirms the signer holds the KEEPER role. Pure-TTL keepers
/// skip this — `ExtendFootprintTtl` is permissionless.
pub async fn assert_keeper_role(
    client: &RpcClient,
    controller_strkey: &str,
    caller_strkey: &str,
) -> Result<()> {
    use crate::stellar::invoke::update_indexes;
    use stellar_xdr::curr::{
        Memo, Preconditions, SequenceNumber, Transaction, TransactionEnvelope, TransactionExt,
        TransactionV1Envelope, VecM,
    };

    let controller_id = contract_id_from_strkey(controller_strkey)?;
    let job = update_indexes(&controller_id, caller_strkey, &[])?;

    let source_account = muxed_account_from_strkey(caller_strkey)?;

    let ops: VecM<stellar_xdr::curr::Operation, 100> = vec![job.op]
        .try_into()
        .map_err(|_| anyhow!("op count overflow"))?;

    let tx = Transaction {
        source_account,
        fee: SIM_FEE_STROOPS,
        seq_num: SequenceNumber(0),
        cond: Preconditions::None,
        memo: Memo::None,
        operations: ops,
        ext: TransactionExt::V0,
    };
    let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures: VecM::default(),
    });

    let sim = client
        .inner()
        .simulate_transaction_envelope(&envelope, Some(stellar_rpc_client::AuthMode::Enforce))
        .await
        .context("simulate update_indexes(empty) for KEEPER role check")?;

    if let Some(err) = sim.error {
        return Err(anyhow!(
            "KEEPER role check failed: simulation rejected with `{err}`. Grant role to {caller_strkey}."
        ));
    }
    Ok(())
}

/// Nominal fee for a simulation-only envelope. The value is irrelevant to the
/// simulator (no tx is submitted), but a sane base keeps the envelope valid.
const SIM_FEE_STROOPS: u32 = 100;
