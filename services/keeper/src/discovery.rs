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
    ControllerPersistentKey, ControllerUserKey, HubAssetKey, OracleAdapterKey, PoolPersistentKey,
    PriceAggregatorPersistentKey,
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
    /// `None` when `governance` is unset.
    pub governance: Option<[u8; 32]>,
    /// `None` when `xoxno_oracle_adapter` is unset.
    pub xoxno_oracle_adapter: Option<[u8; 32]>,
    /// `None` when `price_aggregator` is unset.
    pub price_aggregator: Option<[u8; 32]>,
}

impl ContractIds {
    pub fn resolve(contracts: &ContractsConfig) -> Result<Self> {
        let governance = contracts
            .governance
            .as_deref()
            .map(contract_id_from_strkey)
            .transpose()?;
        let xoxno_oracle_adapter = contracts
            .xoxno_oracle_adapter
            .as_deref()
            .map(contract_id_from_strkey)
            .transpose()?;
        let price_aggregator = contracts
            .price_aggregator
            .as_deref()
            .map(contract_id_from_strkey)
            .transpose()?;
        Ok(Self {
            controller: contract_id_from_strkey(&contracts.controller)?,
            pool_wasm_hash: hash32_from_hex(&contracts.pool_wasm_hash)?,
            flash_receiver: contract_id_from_strkey(&contracts.flash_loan_receiver)?,
            governance,
            xoxno_oracle_adapter,
            price_aggregator,
        })
    }
}

/// Entries discovered during one keeper tick.
fn configured_market_assets(contracts: &ContractsConfig) -> Result<Vec<HubAssetKey>> {
    let mut markets = Vec::with_capacity(contracts.markets.len() + contracts.market_assets.len());
    for market in &contracts.markets {
        markets.push(HubAssetKey {
            hub_id: market.hub_id,
            asset: contract_id_from_strkey(&market.asset)?,
        });
    }
    for asset in &contracts.market_assets {
        markets.push(HubAssetKey {
            hub_id: 1,
            asset: contract_id_from_strkey(asset)?,
        });
    }
    Ok(markets)
}

#[derive(Debug, Default)]
pub struct DiscoverySnapshot {
    pub current_ledger: u32,
    pub assets: Vec<HubAssetKey>,
    /// Protocol persistent entries (markets, spokes, roles, users, adapter index).
    pub persistent_entries: Vec<LedgerEntryQuery>,
    /// Controller/pool/flash/governance/adapter instance entries.
    pub instance_entries: Vec<LedgerEntryQuery>,
    /// WASM code entries (controller, pool, flash, adapter when configured).
    pub wasm_code_entries: Vec<LedgerEntryQuery>,
    /// Account id ceiling; feeds `keeper_account_nonce`.
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

    let instance = client.get_contract_instance(&controller_id).await?;
    let controller_wasm_hash = wasm_hash_from_executable(&instance.executable);
    let pool_id = lookup_scalar(&instance, ControllerInstanceKey::Pool, scval_contract_id)?;
    if pool_id.is_none() {
        warn!(
            target: "keeper.discovery",
            "central pool address missing from controller instance — pool keys skipped this tick"
        );
    }
    let last_spoke_id =
        lookup_scalar(&instance, ControllerInstanceKey::LastSpokeId, scval_u32)?.unwrap_or(0);
    let last_hub_id =
        lookup_scalar(&instance, ControllerInstanceKey::LastHubId, scval_u32)?.unwrap_or(0);

    // AccountNonce is persistent so account creation does not rewrite instance storage.
    let nonce_key = ControllerPersistentKey::AccountNonce.to_ledger_key(&controller_id)?;
    let nonce_rows = client.get_ledger_entries(&[nonce_key]).await?;
    let account_nonce = nonce_rows
        .first()
        .and_then(|row| match row.value.as_ref()? {
            LedgerEntryData::ContractData(cd) => scval_u64(&cd.val),
            _ => None,
        })
        .unwrap_or(0);
    debug!(
        target: "keeper.discovery",
        account_nonce,
        last_spoke_id,
        last_hub_id,
        pool_resolved = pool_id.is_some(),
        "instance read"
    );

    let assets = configured_market_assets(contracts)?;
    let mut persistent_entries = Vec::new();

    let mut pool_rows_present = 0usize;
    let mut pool_rows_total = 0usize;
    for chunk in assets.chunks(chunk_size) {
        let mut keys = Vec::with_capacity(chunk.len() * 3);
        if let Some(aggregator_id) = &ids.price_aggregator {
            for asset in chunk {
                keys.push(
                    PriceAggregatorPersistentKey::AssetOracle(asset.asset)
                        .to_ledger_key(aggregator_id)?,
                );
            }
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
    // All listed markets write Params/State at creation; zero hits ⇒ encoding drift
    // (policy skips value-less rows, so pool TTLs would not extend).
    if pool_rows_total > 0 && pool_rows_present == 0 {
        warn!(
            target: "keeper.discovery",
            assets = assets.len(),
            "no pool Params/State rows resolved — possible PoolKey encoding drift; pool TTLs are NOT being extended"
        );
    }

    if last_spoke_id > 0 {
        for chunk in (1..=last_spoke_id).collect::<Vec<_>>().chunks(chunk_size) {
            let keys = chunk
                .iter()
                .map(|id| ControllerPersistentKey::Spoke(*id).to_ledger_key(&controller_id))
                .collect::<Result<Vec<_>>>()?;
            persistent_entries.extend(client.get_ledger_entries(&keys).await?);
        }
    }

    if last_hub_id > 0 {
        for chunk in (1..=last_hub_id).collect::<Vec<_>>().chunks(chunk_size) {
            let keys = chunk
                .iter()
                .map(|id| ControllerPersistentKey::Hub(*id).to_ledger_key(&controller_id))
                .collect::<Result<Vec<_>>>()?;
            persistent_entries.extend(client.get_ledger_entries(&keys).await?);
        }
    }

    // AccountNonce is protocol-shared; bump it with the rest of the persistent set.
    persistent_entries.extend(nonce_rows);

    persistent_entries.extend(discover_role_keys(client, &controller_id, chunk_size).await?);

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

    // Governance discovery is best-effort: failure must not sink the tick.
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

    // Adapter discovery is best-effort: failure must not sink the tick.
    let mut adapter_instance: Option<LedgerEntryQuery> = None;
    if let Some(adapter_id) = ids.xoxno_oracle_adapter {
        match discover_oracle_adapter(client, &adapter_id, chunk_size).await {
            Ok(adapter) => {
                adapter_instance = Some(adapter.instance);
                persistent_entries.extend(adapter.persistent_entries);
            }
            Err(err) => warn!(
                target: "keeper.discovery",
                error = %err,
                "xoxno-oracle-adapter discovery failed — adapter TTLs skipped this tick"
            ),
        }
    }

    // Price-aggregator instance + code must stay live alongside its persistent
    // AssetOracle rows, or every controller `prices` cross-call fails once the
    // instance archives. Best-effort like governance/adapter discovery.
    let mut aggregator_instance: Option<LedgerEntryQuery> = None;
    if let Some(aggregator_id) = &ids.price_aggregator {
        match client
            .get_ledger_entries(&[contract_instance_key(aggregator_id)])
            .await
        {
            Ok(mut rows) => aggregator_instance = rows.pop(),
            Err(err) => warn!(
                target: "keeper.discovery",
                error = %err,
                "price-aggregator instance discovery failed — aggregator TTLs skipped this tick"
            ),
        }
    }

    let mut instance_keys = Vec::with_capacity(3);
    instance_keys.push(contract_instance_key(&controller_id));
    if let Some(pool) = &pool_id {
        instance_keys.push(contract_instance_key(pool));
    }
    // Flash receiver must stay LAST: wasm-hash harvest uses `.last()`.
    instance_keys.push(contract_instance_key(&ids.flash_receiver));
    let mut instance_entries = client.get_ledger_entries(&instance_keys).await?;

    let mut wasm_keys: Vec<LedgerKey> = vec![contract_code_key(&ids.pool_wasm_hash)];
    if let Some(ctrl_hash) = controller_wasm_hash {
        wasm_keys.push(contract_code_key(&ctrl_hash));
    } else {
        warn!(target: "keeper.discovery", "controller wasm hash unresolved — extending pool wasm only");
    }
    // Live pool code should match networks.json after upgrade_pool.
    // Keep a fallback extend if they diverge.
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
    if let Some(flash_hash) = instance_entries
        .last()
        .and_then(wasm_hash_from_instance_row)
    {
        wasm_keys.push(contract_code_key(&flash_hash));
    }
    // Adapter wasm can archive independently of instance/persistent state.
    if let Some(adapter_hash) = adapter_instance
        .as_ref()
        .and_then(wasm_hash_from_instance_row)
    {
        wasm_keys.push(contract_code_key(&adapter_hash));
    }
    // Same for the price-aggregator code.
    if let Some(aggregator_hash) = aggregator_instance
        .as_ref()
        .and_then(wasm_hash_from_instance_row)
    {
        wasm_keys.push(contract_code_key(&aggregator_hash));
    }
    let wasm_code_entries = client.get_ledger_entries(&wasm_keys).await?;

    // Append after wasm harvest so flash receiver stays LAST in `instance_entries`.
    // Governance instance covers Controller/Owner/Admin/RoleAdmin + instance-tier MinDelay.
    if let Some(gov_instance) = governance_instance {
        instance_entries.push(gov_instance);
    }
    // Adapter instance covers Signers/Threshold/MaxStaleSeconds/Resolution.
    if let Some(adapter) = adapter_instance {
        instance_entries.push(adapter);
    }
    // Price-aggregator instance covers its Ownable owner slot.
    if let Some(aggregator) = aggregator_instance {
        instance_entries.push(aggregator);
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

/// Fallback role list when `ExistingRoles` cannot be read (empty by design).
const DEFAULT_ROLES: [&str; 0] = [];

/// Discover persistent access-control keys (holders + role-admin links).
async fn discover_role_keys(
    client: &RpcClient,
    controller_id: &[u8; 32],
    chunk_size: usize,
) -> Result<Vec<LedgerEntryQuery>> {
    let mut rows: Vec<LedgerEntryQuery> = Vec::new();

    let existing_key = AccessControlPersistentKey::ExistingRoles.to_ledger_key(controller_id)?;
    let existing_rows = client.get_ledger_entries(&[existing_key]).await?;
    let roles = extract_existing_roles(&existing_rows)
        .unwrap_or_else(|| DEFAULT_ROLES.iter().map(|s| s.to_string()).collect());
    rows.extend(existing_rows);

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

struct GovernanceEntries {
    /// Instance covers Controller/Owner/Admin/RoleAdmin + instance-tier MinDelay.
    instance: LedgerEntryQuery,
    role_entries: Vec<LedgerEntryQuery>,
}

/// Governance instance + persistent role keys.
///
/// `MinDelay` is instance-tier — one instance bump covers it. Timelock
/// `OperationLedger(BytesN<32>)` keys are persistent but not enumerable
/// (keccak256 op id from schedule events); they resolve within `min_delay` ≪ TTL
/// and are intentionally skipped.
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

    // Same access-control encoding as the controller.
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

struct OracleAdapterEntries {
    /// Instance covers Signers/Threshold/MaxStaleSeconds/Resolution.
    instance: LedgerEntryQuery,
    persistent_entries: Vec<LedgerEntryQuery>,
}

/// Adapter instance + enumerable persistent index/price state.
///
/// Asset/feed index and price state (`CurrentAggregate`, `History`,
/// `FeedMapping`, `LatestSubmission`) are PERSISTENT — TTL renews only on write,
/// so idle feeds archive and trap reads. Walks on-chain `AssetCount`/`FeedCount`
/// + `AssetAt`/`FeedAt` slots; raw slot ScVals are passed through (no hardcodes).
///
/// `LatestSubmission` and `SignerFeeds` key off INSTANCE `Signers`.
async fn discover_oracle_adapter(
    client: &RpcClient,
    adapter_id: &[u8; 32],
    chunk_size: usize,
) -> Result<OracleAdapterEntries> {
    let chunk = chunk_size.max(1);

    // Instance: bump target + Signers for LatestSubmission keys.
    let instance_rows = client
        .get_ledger_entries(&[contract_instance_key(adapter_id)])
        .await?;
    let instance = instance_rows
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("oracle-adapter instance query returned no row"))?;
    if instance.value.is_none() {
        warn!(
            target: "keeper.discovery",
            adapter = %stellar_strkey::Contract(*adapter_id),
            "oracle-adapter instance entry absent — persistent coverage skipped this tick"
        );
        return Ok(OracleAdapterEntries {
            instance,
            persistent_entries: Vec::new(),
        });
    }
    let signers = signers_from_instance(&instance);

    let mut persistent_entries: Vec<LedgerEntryQuery> = Vec::new();

    let count_keys = vec![
        OracleAdapterKey::AssetCount.to_ledger_key(adapter_id)?,
        OracleAdapterKey::FeedCount.to_ledger_key(adapter_id)?,
    ];
    let count_rows = client.get_ledger_entries(&count_keys).await?;
    let asset_count = count_rows.first().and_then(extract_u32).unwrap_or(0);
    let feed_count = count_rows.get(1).and_then(extract_u32).unwrap_or(0);
    persistent_entries.extend(count_rows);

    let mut derived_keys: Vec<LedgerKey> = Vec::new();

    // `remove_signer` reads SignerFeeds; idle signer indexes must not archive.
    for signer in &signers {
        derived_keys
            .push(OracleAdapterKey::SignerFeeds(signer.clone()).to_ledger_key(adapter_id)?);
    }

    for id_chunk in (0..asset_count).collect::<Vec<_>>().chunks(chunk) {
        let keys = id_chunk
            .iter()
            .map(|i| OracleAdapterKey::AssetAt(*i).to_ledger_key(adapter_id))
            .collect::<Result<Vec<_>>>()?;
        for row in client.get_ledger_entries(&keys).await? {
            if let Some(asset) = contract_data_scval(&row) {
                derived_keys.push(
                    OracleAdapterKey::AssetIndex(asset.clone()).to_ledger_key(adapter_id)?,
                );
                derived_keys.push(OracleAdapterKey::FeedMapping(asset).to_ledger_key(adapter_id)?);
            }
            persistent_entries.push(row);
        }
    }

    for id_chunk in (0..feed_count).collect::<Vec<_>>().chunks(chunk) {
        let keys = id_chunk
            .iter()
            .map(|i| OracleAdapterKey::FeedAt(*i).to_ledger_key(adapter_id))
            .collect::<Result<Vec<_>>>()?;
        for row in client.get_ledger_entries(&keys).await? {
            if let Some(feed) = contract_data_scval(&row) {
                derived_keys
                    .push(OracleAdapterKey::FeedIndex(feed.clone()).to_ledger_key(adapter_id)?);
                derived_keys
                    .push(OracleAdapterKey::FeedOwner(feed.clone()).to_ledger_key(adapter_id)?);
                derived_keys.push(
                    OracleAdapterKey::CurrentAggregate(feed.clone())
                        .to_ledger_key(adapter_id)?,
                );
                derived_keys
                    .push(OracleAdapterKey::History(feed.clone()).to_ledger_key(adapter_id)?);
                for signer in &signers {
                    derived_keys.push(
                        OracleAdapterKey::LatestSubmission(feed.clone(), signer.clone())
                            .to_ledger_key(adapter_id)?,
                    );
                }
            }
            persistent_entries.push(row);
        }
    }

    for key_chunk in derived_keys.chunks(chunk) {
        persistent_entries.extend(client.get_ledger_entries(key_chunk).await?);
    }

    debug!(
        target: "keeper.discovery",
        assets = asset_count,
        feeds = feed_count,
        signers = signers.len(),
        adapter_entries = persistent_entries.len(),
        "oracle-adapter keys discovered"
    );
    Ok(OracleAdapterEntries {
        instance,
        persistent_entries,
    })
}

fn contract_data_scval(row: &LedgerEntryQuery) -> Option<ScVal> {
    match row.value.as_ref()? {
        LedgerEntryData::ContractData(cd) => Some(cd.val.clone()),
        _ => None,
    }
}

/// INSTANCE `Signers` (`Vec<Address>`); empty if unset.
fn signers_from_instance(instance: &LedgerEntryQuery) -> Vec<ScAddress> {
    let Some(LedgerEntryData::ContractData(cd)) = instance.value.as_ref() else {
        return Vec::new();
    };
    let ScVal::ContractInstance(inst) = &cd.val else {
        return Vec::new();
    };
    let Some(storage) = &inst.storage else {
        return Vec::new();
    };
    let Some(needle) = signers_needle() else {
        return Vec::new();
    };
    for ScMapEntry { key, val } in storage.0.iter() {
        if key == &needle {
            let ScVal::Vec(Some(vec)) = val else {
                return Vec::new();
            };
            return vec
                .0
                .iter()
                .filter_map(|v| match v {
                    ScVal::Address(addr) => Some(addr.clone()),
                    _ => None,
                })
                .collect();
        }
    }
    Vec::new()
}

/// INSTANCE lookup key: `Vec[Symbol("Signers")]`.
fn signers_needle() -> Option<ScVal> {
    let symbol = ScSymbol(StringM::<32>::try_from("Signers").ok()?);
    let vec = vec![ScVal::Symbol(symbol)].try_into().ok()?;
    Some(ScVal::Vec(Some(stellar_xdr::curr::ScVec(vec))))
}

/// Per-user keys for `1..=account_nonce` (meta/supply/borrow/delegates).
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
        let mut keys = Vec::with_capacity(id_chunk.len() * 4);
        for &id in id_chunk {
            keys.push(ControllerUserKey::AccountMeta(id).to_ledger_key(controller_id)?);
            keys.push(ControllerUserKey::SupplyPositions(id).to_ledger_key(controller_id)?);
            keys.push(ControllerUserKey::BorrowPositions(id).to_ledger_key(controller_id)?);
            keys.push(ControllerUserKey::Delegates(id).to_ledger_key(controller_id)?);
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

pub fn self_check(contracts: &ContractsConfig) -> Result<Vec<HubAssetKey>> {
    configured_market_assets(contracts)
}

/// Boot preflight: signer can simulate `update_indexes` (caller auth only).
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

/// Nominal fee for sim-only envelopes (not submitted).
const SIM_FEE_STROOPS: u32 = 100;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_accepts_testnet_governance_address() {
        let contracts = ContractsConfig {
            controller: "CBSCWXCIAASFR2F2332D2I7C6VWUJZKUW4ONOZR2LZ32KOZ5UZVNJ3LA".into(),
            pool_wasm_hash: "a1e7db9b32626c8d4c57343c50407956ea1b642054bf6aee0a613da06359a6fa"
                .into(),
            flash_loan_receiver: "CCYDZ6SLHGZKBJF3MNKRK2QPITSVTHL5NYWKWWPMNSOTW4HHCK32JNLZ".into(),
            markets: Vec::new(),
            market_assets: Vec::new(),
            governance: Some("CCGAETDFZNTJYNOFRC3DR3KZCDZFANBEN2CJSBTOGTLVJPRAFPF7DWMH".into()),
            xoxno_oracle_adapter: None,
            price_aggregator: None,
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
            markets: Vec::new(),
            market_assets: Vec::new(),
            governance: None,
            xoxno_oracle_adapter: None,
            price_aggregator: None,
        };
        let ids = ContractIds::resolve(&contracts).unwrap();
        assert!(ids.governance.is_none());
    }
}
