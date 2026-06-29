//! Discovery of storage, instance, and code entries the keeper may renew.

use anyhow::{anyhow, Context, Result};
use stellar_xdr::curr::{
    ContractExecutable, ContractId, Hash, LedgerEntryData, LedgerKey, ScAddress,
    ScContractInstance, ScMapEntry, ScSymbol, ScVal, StringM,
};
use tracing::{debug, info, warn};

use crate::config::{ContractsConfig, ScheduleConfig};
use crate::keys::{
    contract_code_key, contract_instance_key, AccessControlPersistentKey, ControllerInstanceKey,
    ControllerPersistentKey, ControllerUserKey, PoolPersistentKey,
};
use crate::stellar::client::{
    contract_id_from_strkey, hash32_from_hex, LedgerEntryQuery, RpcClient,
};

/// Contract ids parsed once from config.
#[derive(Debug, Clone, Copy)]
pub struct ContractIds {
    pub controller: [u8; 32],
    pub pool_wasm_hash: [u8; 32],
    pub flash_receiver: [u8; 32],
    /// Governance contract id; `None` when no `governance` address is configured.
    pub governance: Option<[u8; 32]>,
}

impl ContractIds {
    pub fn resolve(contracts: &ContractsConfig) -> Result<Self> {
        let governance = contracts
            .governance
            .as_deref()
            .map(contract_id_from_strkey)
            .transpose()?;
        Ok(Self {
            controller: contract_id_from_strkey(&contracts.controller)?,
            pool_wasm_hash: hash32_from_hex(&contracts.pool_wasm_hash)?,
            flash_receiver: contract_id_from_strkey(&contracts.flash_loan_receiver)?,
            governance,
        })
    }
}

/// Entries discovered during one keeper tick.
fn configured_market_assets(contracts: &ContractsConfig) -> Result<Vec<[u8; 32]>> {
    contracts
        .market_assets
        .iter()
        .map(|asset| contract_id_from_strkey(asset))
        .collect()
}

#[derive(Debug, Default)]
pub struct DiscoverySnapshot {
    pub current_ledger: u32,
    pub assets: Vec<[u8; 32]>,
    /// Persistent protocol entries: per-asset, spoke, role keys, the per-user
    /// account keys (when `scan_users`), and governance role keys.
    pub persistent_entries: Vec<LedgerEntryQuery>,
    /// Controller, central pool, flash receiver, and (when configured)
    /// governance instance entries.
    pub instance_entries: Vec<LedgerEntryQuery>,
    /// WASM code entries for controller, pool, and flash receiver.
    pub wasm_code_entries: Vec<LedgerEntryQuery>,
    /// Account id ceiling exposed as the `keeper_account_nonce` metric.
    pub account_nonce: u64,
}

pub async fn snapshot(
    client: &RpcClient,
    ids: &ContractIds,
    contracts: &ContractsConfig,
    schedule: &ScheduleConfig,
) -> Result<DiscoverySnapshot> {
    let chunk_size = schedule.asset_chunk.max(1);
    let controller_id = ids.controller;

    let current_ledger = client.latest_ledger().await?;
    info!(target: "keeper.discovery", current_ledger, "tick start");

    // -- Controller instance: wasm hash + pool address + AccountNonce + spoke ceiling --
    let instance = client.get_contract_instance(&controller_id).await?;
    let controller_wasm_hash = wasm_hash_from_executable(&instance.executable);
    let pool_id = lookup_scalar(&instance, ControllerInstanceKey::Pool, scval_contract_id)?;
    if pool_id.is_none() {
        warn!(
            target: "keeper.discovery",
            "central pool address missing from controller instance — pool keys skipped this tick"
        );
    }
    let account_nonce =
        lookup_scalar(&instance, ControllerInstanceKey::AccountNonce, scval_u64)?.unwrap_or(0);
    let last_spoke_id = lookup_scalar(
        &instance,
        ControllerInstanceKey::LastSpokeId,
        scval_u32,
    )?
    .unwrap_or(0);
    debug!(
        target: "keeper.discovery",
        account_nonce,
        last_spoke_id,
        pool_resolved = pool_id.is_some(),
        "instance read"
    );

    // -- Pool list (persistent) --
    let assets = configured_market_assets(contracts)?;
    let mut persistent_entries = Vec::new();

    // -- Per-asset persistent state: controller Market plus the central pool's
    //    asset-keyed Params + State entries --
    let mut pool_rows_present = 0usize;
    let mut pool_rows_total = 0usize;
    for chunk in assets.chunks(chunk_size) {
        let mut keys = Vec::with_capacity(chunk.len() * 3);
        for asset in chunk {
            keys.push(ControllerPersistentKey::Market(*asset).to_ledger_key(&controller_id)?);
        }
        if let Some(pool) = &pool_id {
            for asset in chunk {
                keys.push(PoolPersistentKey::Params(*asset).to_ledger_key(pool)?);
                keys.push(PoolPersistentKey::State(*asset).to_ledger_key(pool)?);
                pool_rows_total += 2;
            }
        }
        for row in client.get_ledger_entries(&keys).await? {
            if row_belongs_to(&row, pool_id.as_ref()) && row.value.is_some() {
                pool_rows_present += 1;
            }
            persistent_entries.push(row);
        }
    }
    // Encoding-drift alarm: every market writes Params/State at creation, so an
    // all-absent pool key set with listed assets means the keeper is bumping
    // nothing on the pool (policy skips value-less rows) — alert loudly.
    if pool_rows_total > 0 && pool_rows_present == 0 {
        warn!(
            target: "keeper.discovery",
            assets = assets.len(),
            "no pool Params/State rows resolved — possible PoolKey encoding drift; pool TTLs are NOT being extended"
        );
    }

    // -- Spoke category sweep (1..=ceiling) --
    if last_spoke_id > 0 {
        for chunk in (1..=last_spoke_id)
            .collect::<Vec<_>>()
            .chunks(chunk_size)
        {
            let keys = chunk
                .iter()
                .map(|id| ControllerPersistentKey::Spoke(*id).to_ledger_key(&controller_id))
                .collect::<Result<Vec<_>>>()?;
            persistent_entries.extend(client.get_ledger_entries(&keys).await?);
        }
    }

    // -- Access-control role keys --
    persistent_entries.extend(discover_role_keys(client, &controller_id, chunk_size).await?);

    // -- Per-user account keys (1..=AccountNonce) --
    if schedule.scan_users && account_nonce > 0 {
        persistent_entries.extend(
            discover_user_keys(
                client,
                &controller_id,
                account_nonce,
                schedule.max_accounts_scan,
                chunk_size,
            )
            .await?,
        );
    }

    // -- Governance coverage (instance + MinDelay-via-instance + role keys) --
    // A read failure must not sink the whole tick: warn and carry on with the
    // controller/pool surface already gathered.
    let mut governance_instance: Option<LedgerEntryQuery> = None;
    if let Some(governance_id) = ids.governance {
        match discover_governance(client, &governance_id, chunk_size).await {
            Ok(gov) => {
                governance_instance = Some(gov.instance);
                persistent_entries.extend(gov.role_entries);
            }
            Err(err) => warn!(
                target: "keeper.discovery",
                error = %err,
                "governance discovery failed — governance TTLs skipped this tick"
            ),
        }
    }

    // -- Instance entries (controller + central pool + flash receiver) --
    let mut instance_keys = Vec::with_capacity(3);
    instance_keys.push(contract_instance_key(&controller_id));
    if let Some(pool) = &pool_id {
        instance_keys.push(contract_instance_key(pool));
    }
    // Keep the flash receiver LAST: the wasm-hash harvest below relies on it.
    instance_keys.push(contract_instance_key(&ids.flash_receiver));
    let mut instance_entries = client.get_ledger_entries(&instance_keys).await?;

    // -- Wasm code entries (pool template + controller + live pool + flash receiver) --
    let mut wasm_keys: Vec<LedgerKey> = vec![contract_code_key(&ids.pool_wasm_hash)];
    if let Some(ctrl_hash) = controller_wasm_hash {
        wasm_keys.push(contract_code_key(&ctrl_hash));
    } else {
        warn!(target: "keeper.discovery", "controller wasm hash unresolved — pool template extend only");
    }
    // The live pool executable can diverge from the configured template hash
    // after an on-chain `upgrade_pool`; harvest it so the running code entry
    // stays bumped even when the config lags.
    if pool_id.is_some() {
        if let Some(live_pool_hash) = instance_entries
            .get(1)
            .and_then(wasm_hash_from_instance_row)
        {
            if live_pool_hash != ids.pool_wasm_hash {
                wasm_keys.push(contract_code_key(&live_pool_hash));
            }
        }
    }
    // The flash-receiver wasm hash lives in the instance entry we just read.
    if let Some(flash_hash) = instance_entries
        .last()
        .and_then(wasm_hash_from_instance_row)
    {
        wasm_keys.push(contract_code_key(&flash_hash));
    }
    let wasm_code_entries = client.get_ledger_entries(&wasm_keys).await?;

    // Append the governance instance only now: the flash-receiver wasm harvest
    // above relies on the flash receiver staying LAST in `instance_entries`.
    // One governance instance bump covers `Controller`, ownable `Owner`,
    // access_control `Admin` + `RoleAdmin`, and the timelock `MinDelay`
    // (instance-tier — see `keys.rs` governance notes).
    if let Some(gov_instance) = governance_instance {
        instance_entries.push(gov_instance);
    }

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
const DEFAULT_ROLES: [&str; 0] = [];

/// Discover persistent access-control keys, including role-admin links.
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

    // Per-role RoleAccountsCount and RoleAdmin.
    let mut role_keys = Vec::with_capacity(roles.len() * 2);
    for role in &roles {
        role_keys.push(
            AccessControlPersistentKey::RoleAccountsCount(role.clone())
                .to_ledger_key(controller_id)?,
        );
        role_keys.push(
            AccessControlPersistentKey::RoleAdmin(role.clone()).to_ledger_key(controller_id)?,
        );
    }
    let role_rows = client.get_ledger_entries(&role_keys).await?;
    let counts: Vec<(String, u32)> = roles
        .iter()
        .cloned()
        .zip(
            role_rows
                .chunks(2)
                .map(|rows| extract_u32(&rows[0]).unwrap_or(0)),
        )
        .collect();
    rows.extend(role_rows);

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

/// Governance entries discovered when a governance contract is configured.
struct GovernanceEntries {
    /// The governance instance entry (covers `Controller`, `Owner`, `Admin`,
    /// `RoleAdmin`, and the instance-tier timelock `MinDelay`).
    instance: LedgerEntryQuery,
    /// Persistent access-control role-holder keys.
    role_entries: Vec<LedgerEntryQuery>,
}

/// Discover the governance instance entry plus its persistent role keys.
///
/// `MinDelay` needs no standalone key: it is instance-tier in
/// stellar-governance, so the instance bump covers it. The timelock
/// `OperationLedger(BytesN<32>)` per-op keys are persistent but NOT enumerable
/// on-chain (the id is a keccak256 hash from the schedule event); they are
/// transient (resolved within `min_delay` ≪ TTL) and intentionally skipped.
async fn discover_governance(
    client: &RpcClient,
    governance_id: &[u8; 32],
    chunk_size: usize,
) -> Result<GovernanceEntries> {
    let instance_rows = client
        .get_ledger_entries(&[contract_instance_key(governance_id)])
        .await?;
    let instance = instance_rows
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("governance instance query returned no row"))?;
    if instance.value.is_none() {
        warn!(
            target: "keeper.discovery",
            governance = %stellar_strkey::Contract(*governance_id),
            "governance instance entry absent — instance bump will be skipped"
        );
    }

    // Reuse the controller role-key discovery against the governance id; the
    // access-control encoding is identical across contracts.
    let role_entries = discover_role_keys(client, governance_id, chunk_size).await?;

    debug!(
        target: "keeper.discovery",
        role_entries = role_entries.len(),
        "governance keys discovered"
    );
    Ok(GovernanceEntries {
        instance,
        role_entries,
    })
}

/// Discover the per-user controller keys for accounts `1..=account_nonce`.
///
/// Builds the three per-account keys (`AccountMeta`, `SupplyPositions`,
/// `BorrowPositions`) for every id.
async fn discover_user_keys(
    client: &RpcClient,
    controller_id: &[u8; 32],
    account_nonce: u64,
    max_accounts_scan: u64,
    chunk_size: usize,
) -> Result<Vec<LedgerEntryQuery>> {
    let scan_ceiling = account_nonce.min(max_accounts_scan.max(1));
    if account_nonce > scan_ceiling {
        warn!(
            target: "keeper.discovery",
            account_nonce,
            max_accounts_scan,
            dropped_from = scan_ceiling + 1,
            dropped_to = account_nonce,
            "AccountNonce exceeds max_accounts_scan — per-user scan TRUNCATED; \
             ids {}..={} are NOT being bumped this tick (raise schedule.max_accounts_scan)",
            scan_ceiling + 1,
            account_nonce
        );
    }

    let mut rows: Vec<LedgerEntryQuery> = Vec::new();
    let chunk = chunk_size.max(1);
    let ids: Vec<u64> = (1..=scan_ceiling).collect();

    for id_chunk in ids.chunks(chunk) {
        let mut keys = Vec::with_capacity(id_chunk.len() * 3);
        for &id in id_chunk {
            keys.push(ControllerUserKey::AccountMeta(id).to_ledger_key(controller_id)?);
            keys.push(ControllerUserKey::SupplyPositions(id).to_ledger_key(controller_id)?);
            keys.push(ControllerUserKey::BorrowPositions(id).to_ledger_key(controller_id)?);
        }
        rows.extend(client.get_ledger_entries(&keys).await?);
    }

    debug!(
        target: "keeper.discovery",
        scanned = scan_ceiling,
        per_user_entries = rows.len(),
        "per-user account keys discovered"
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

/// True when a row's ledger key targets the given contract id.
fn row_belongs_to(row: &LedgerEntryQuery, contract_id: Option<&[u8; 32]>) -> bool {
    let Some(id) = contract_id else {
        return false;
    };
    match &row.key {
        LedgerKey::ContractData(cd) => {
            matches!(&cd.contract, ScAddress::Contract(ContractId(Hash(b))) if b == id)
        }
        _ => false,
    }
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

fn scval_contract_id(val: &ScVal) -> Option<[u8; 32]> {
    match val {
        ScVal::Address(ScAddress::Contract(ContractId(Hash(bytes)))) => Some(*bytes),
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



pub fn self_check(contracts: &ContractsConfig) -> Result<Vec<[u8; 32]>> {
    configured_market_assets(contracts)
}

/// Verifies the signer can simulate `update_indexes` (caller auth only).
pub async fn assert_update_indexes_simulation(
    client: &RpcClient,
    controller_strkey: &str,
    caller_strkey: &str,
) -> Result<()> {
    use crate::stellar::invoke::update_indexes;
    use crate::stellar::tx::build_envelope;

    let controller_id = contract_id_from_strkey(controller_strkey)?;
    let job = update_indexes(&controller_id, caller_strkey, &[])?;
    let envelope = build_envelope(caller_strkey, 0, SIM_FEE_STROOPS, job.op, None)?;

    let sim = client
        .inner()
        .simulate_transaction_envelope(&envelope, Some(stellar_rpc_client::AuthMode::Enforce))
        .await
        .context("simulate update_indexes(empty) for boot preflight")?;

    if let Some(err) = sim.error {
        return Err(anyhow!(
            "update_indexes simulation failed with `{err}` for signer {caller_strkey}."
        ));
    }
    Ok(())
}

/// Nominal fee for a simulation-only envelope. The value is irrelevant to the
/// simulator (no tx is submitted), but a sane base keeps the envelope valid.
const SIM_FEE_STROOPS: u32 = 100;

#[cfg(test)]
mod tests {
    use super::*;

    /// The deployed testnet governance address must resolve through the same
    /// strkey decoder the keeper uses for the controller.
    #[test]
    fn resolve_accepts_testnet_governance_address() {
        let contracts = ContractsConfig {
            controller: "CBSCWXCIAASFR2F2332D2I7C6VWUJZKUW4ONOZR2LZ32KOZ5UZVNJ3LA".into(),
            pool_wasm_hash: "a1e7db9b32626c8d4c57343c50407956ea1b642054bf6aee0a613da06359a6fa"
                .into(),
            flash_loan_receiver: "CCYDZ6SLHGZKBJF3MNKRK2QPITSVTHL5NYWKWWPMNSOTW4HHCK32JNLZ".into(),
            market_assets: Vec::new(),
            governance: Some("CCGAETDFZNTJYNOFRC3DR3KZCDZFANBEN2CJSBTOGTLVJPRAFPF7DWMH".into()),
        };
        let ids = ContractIds::resolve(&contracts).unwrap();
        assert!(ids.governance.is_some());
    }

    #[test]
    fn resolve_governance_none_when_unset() {
        let contracts = ContractsConfig {
            controller: "CBSCWXCIAASFR2F2332D2I7C6VWUJZKUW4ONOZR2LZ32KOZ5UZVNJ3LA".into(),
            pool_wasm_hash: "a1e7db9b32626c8d4c57343c50407956ea1b642054bf6aee0a613da06359a6fa"
                .into(),
            flash_loan_receiver: "CCYDZ6SLHGZKBJF3MNKRK2QPITSVTHL5NYWKWWPMNSOTW4HHCK32JNLZ".into(),
            market_assets: Vec::new(),
            governance: None,
        };
        let ids = ContractIds::resolve(&contracts).unwrap();
        assert!(ids.governance.is_none());
    }

}
