use anyhow::{anyhow, Context, Result};
use std::sync::atomic::{AtomicU64, Ordering};
use stellar_rpc_client::AuthMode;
use stellar_xdr::{
    ContractExecutable, ContractId, Hash, LedgerEntryData, LedgerKey, ScAddress,
    ScContractInstance, ScMapEntry, ScSymbol, ScVal, StringM,
};
use tracing::{debug, info, warn};

use crate::config::{ContractsConfig, ScheduleConfig};
use crate::keys::{
    contract_code_key, contract_instance_key, AccessControlPersistentKey, AggregatorPriceKey,
    ControllerInstanceKey, ControllerPersistentKey, ControllerUserKey, HubAssetKey,
    OracleAdapterKey, PoolPersistentKey, PositionNftInstanceKey, PositionNftUserKey,
    PriceAggregatorInstanceKey, PriceAggregatorPersistentKey,
};
use crate::stellar::client::{
    contract_id_from_strkey, hash32_from_hex, LedgerEntryQuery, RpcClient,
};

#[derive(Debug, Clone, Copy)]
pub struct ContractIds {
    pub controller: [u8; 32],
    pub pool_wasm_hash: [u8; 32],
    pub flash_receiver: Option<[u8; 32]>,

    pub governance: Option<[u8; 32]>,

    pub xoxno_oracle_adapter: Option<[u8; 32]>,

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
            flash_receiver: contracts
                .flash_loan_receiver
                .as_deref()
                .map(contract_id_from_strkey)
                .transpose()?,
            governance,
            xoxno_oracle_adapter,
            price_aggregator,
        })
    }
}

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

    pub persistent_entries: Vec<LedgerEntryQuery>,

    pub instance_entries: Vec<LedgerEntryQuery>,

    pub wasm_code_entries: Vec<LedgerEntryQuery>,

    /// Highest position-NFT token id minted so far, i.e. the largest account id
    /// that can exist. Zero when the NFT address or counter cannot be read.
    pub max_account_id: u64,

    /// Pool and position-NFT ids, read from the controller instance rather than
    /// configured. Carried so metric labels can name them instead of falling
    /// back to a hex prefix.
    pub pool_id: Option<[u8; 32]>,
    pub position_nft_id: Option<[u8; 32]>,
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

    // Account ids are position-NFT token ids. The controller keeps no counter of
    // its own, so the id ceiling comes from the NFT's sequential counter, which
    // holds the NEXT free id. Ids are never reused, so scanning 1..=max_account_id
    // covers every account that has ever existed.
    let position_nft_id = lookup_scalar(
        &instance,
        ControllerInstanceKey::PositionNft,
        scval_contract_id,
    )?;
    let max_account_id = match position_nft_id {
        Some(nft_id) => {
            // Read through the ledger-entry path, not get_contract_instance:
            // an archived NFT instance is exactly the state this tick has to
            // repair, and a hard error here would abort discovery before
            // plan_restores ever sees the row, so the contract could never be
            // restored. A missing instance yields no counter, which stops the
            // user scan for this tick while the restore is planned below.
            let rows = client
                .get_ledger_entries(&[contract_instance_key(&nft_id)])
                .await?;
            let next_token_id = rows
                .first()
                .and_then(instance_from_row)
                .map(|inst| {
                    lookup_instance_scalar(
                        inst,
                        PositionNftInstanceKey::TokenIdCounter.variant_name(),
                        scval_u32,
                    )
                })
                .transpose()?
                .flatten();
            match next_token_id {
                Some(counter) => max_account_id_from_counter(counter),
                None => {
                    warn!(
                        target: "keeper.discovery",
                        "position-NFT instance or its token counter is unreadable (archived?) — user account keys skipped this tick; a restore is planned if the instance is archived"
                    );
                    0
                }
            }
        }
        None => {
            warn!(
                target: "keeper.discovery",
                "position-NFT address missing from controller instance — user account keys skipped this tick"
            );
            0
        }
    };
    debug!(
        target: "keeper.discovery",
        max_account_id,
        last_spoke_id,
        last_hub_id,
        pool_resolved = pool_id.is_some(),
        position_nft_resolved = position_nft_id.is_some(),
        "instance read"
    );

    let assets = configured_market_assets(contracts)?;
    let mut persistent_entries = Vec::new();

    let mut pool_rows_present = 0usize;
    let mut pool_rows_total = 0usize;
    for chunk in assets.chunks(chunk_size) {
        let mut keys = Vec::with_capacity(chunk.len() * 2);
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

    // The aggregator's own `OracleKeys` index is the authoritative set of stored
    // `PriceKey`s. Reading it covers `Ref` rows, which no market-address list can
    // produce, and drops rows for assets the aggregator never registered.
    // Falling back to the configured markets keeps a stale or unreadable index
    // from silently dropping oracle coverage to nothing.
    if let Some(aggregator_id) = &ids.price_aggregator {
        let registered = match client.get_contract_instance(aggregator_id).await {
            Ok(agg_instance) => lookup_oracle_keys(&agg_instance)?,
            Err(e) => {
                warn!(
                    target: "keeper.discovery",
                    error = ?e,
                    "price-aggregator instance unreadable; falling back to configured markets"
                );
                None
            }
        };
        let oracle_keys = registered.unwrap_or_else(|| {
            warn!(
                target: "keeper.discovery",
                "price-aggregator OracleKeys index missing; falling back to configured markets"
            );
            assets
                .iter()
                .map(|a| AggregatorPriceKey::Token(a.asset))
                .collect()
        });
        let mut oracle_rows_present = 0usize;
        let oracle_rows_total = oracle_keys.len();
        for chunk in oracle_keys.chunks(chunk_size) {
            let mut keys = Vec::with_capacity(chunk.len());
            for key in chunk {
                keys.push(
                    PriceAggregatorPersistentKey::Oracle(key.clone())
                        .to_ledger_key(aggregator_id)?,
                );
            }
            for row in client.get_ledger_entries(&keys).await? {
                if row.value.is_some() {
                    oracle_rows_present += 1;
                }
                persistent_entries.push(row);
            }
        }
        info!(
            target: "keeper.discovery",
            oracle_rows_total,
            oracle_rows_present,
            "price-aggregator oracle rows"
        );
        if oracle_rows_total > 0 && oracle_rows_present == 0 {
            warn!(
                target: "keeper.discovery",
                "every price-aggregator oracle row read back absent — the key encoding \
                 no longer matches AggregatorKey::Oracle(PriceKey)"
            );
        }
    }

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

    persistent_entries.extend(discover_role_keys(client, &controller_id, chunk_size).await?);

    if schedule.scan_users && max_account_id > 0 {
        persistent_entries.extend(
            discover_user_keys(
                client,
                &controller_id,
                position_nft_id,
                max_account_id,
                schedule.max_accounts_scan,
                chunk_size,
            )
            .await?,
        );
    }

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

    let InstancePlan {
        keys: instance_keys,
        pool_row,
        flash_row,
        nft_row,
    } = plan_instance_keys(
        &controller_id,
        pool_id.as_ref(),
        ids.flash_receiver.as_ref(),
        position_nft_id.as_ref(),
    );
    let mut instance_entries = client.get_ledger_entries(&instance_keys).await?;

    let mut wasm_keys: Vec<LedgerKey> = vec![contract_code_key(&ids.pool_wasm_hash)];
    if let Some(ctrl_hash) = controller_wasm_hash {
        wasm_keys.push(contract_code_key(&ctrl_hash));
    } else {
        warn!(target: "keeper.discovery", "controller wasm hash unresolved — extending pool wasm only");
    }

    if let Some(live_pool_hash) = pool_row
        .and_then(|i| instance_entries.get(i))
        .and_then(wasm_hash_from_instance_row)
    {
        if live_pool_hash != ids.pool_wasm_hash {
            wasm_keys.push(contract_code_key(&live_pool_hash));
        }
    }
    if let Some(flash_hash) = flash_row
        .and_then(|i| instance_entries.get(i))
        .and_then(wasm_hash_from_instance_row)
    {
        wasm_keys.push(contract_code_key(&flash_hash));
    }
    if let Some(nft_hash) = nft_row
        .and_then(|i| instance_entries.get(i))
        .and_then(wasm_hash_from_instance_row)
    {
        wasm_keys.push(contract_code_key(&nft_hash));
    }

    if let Some(adapter_hash) = adapter_instance
        .as_ref()
        .and_then(wasm_hash_from_instance_row)
    {
        wasm_keys.push(contract_code_key(&adapter_hash));
    }

    if let Some(aggregator_hash) = aggregator_instance
        .as_ref()
        .and_then(wasm_hash_from_instance_row)
    {
        wasm_keys.push(contract_code_key(&aggregator_hash));
    }
    let wasm_code_entries = client.get_ledger_entries(&wasm_keys).await?;

    if let Some(gov_instance) = governance_instance {
        instance_entries.push(gov_instance);
    }

    if let Some(adapter) = adapter_instance {
        instance_entries.push(adapter);
    }

    if let Some(aggregator) = aggregator_instance {
        instance_entries.push(aggregator);
    }

    Ok(DiscoverySnapshot {
        current_ledger,
        assets,
        persistent_entries,
        instance_entries,
        pool_id,
        position_nft_id,
        wasm_code_entries,
        max_account_id,
    })
}

const DEFAULT_ROLES: [&str; 0] = [];

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
    instance: LedgerEntryQuery,
    role_entries: Vec<LedgerEntryQuery>,
}

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
    instance: LedgerEntryQuery,
    persistent_entries: Vec<LedgerEntryQuery>,
}

async fn discover_oracle_adapter(
    client: &RpcClient,
    adapter_id: &[u8; 32],
    chunk_size: usize,
) -> Result<OracleAdapterEntries> {
    let chunk = chunk_size.max(1);

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

    for signer in &signers {
        derived_keys.push(OracleAdapterKey::SignerFeeds(signer.clone()).to_ledger_key(adapter_id)?);
    }

    for id_chunk in (0..asset_count).collect::<Vec<_>>().chunks(chunk) {
        let keys = id_chunk
            .iter()
            .map(|i| OracleAdapterKey::AssetAt(*i).to_ledger_key(adapter_id))
            .collect::<Result<Vec<_>>>()?;
        for row in client.get_ledger_entries(&keys).await? {
            if let Some(asset) = contract_data_scval(&row) {
                derived_keys
                    .push(OracleAdapterKey::AssetIndex(asset.clone()).to_ledger_key(adapter_id)?);
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
                    OracleAdapterKey::CurrentAggregate(feed.clone()).to_ledger_key(adapter_id)?,
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

fn signers_needle() -> Option<ScVal> {
    let symbol = ScSymbol(StringM::<32>::try_from("Signers").ok()?);
    let vec = vec![ScVal::Symbol(symbol)].try_into().ok()?;
    Some(ScVal::Vec(Some(stellar_xdr::ScVec(vec))))
}

/// Rotating start of the per-user scan window, carried across ticks.
///
/// A fixed `1..=max_accounts_scan` prefix would permanently exclude every
/// account created past the cap, since account ids only ever increase.
static USER_SCAN_CURSOR: AtomicU64 = AtomicU64::new(1);

async fn discover_user_keys(
    client: &RpcClient,
    controller_id: &[u8; 32],
    position_nft_id: Option<[u8; 32]>,
    max_account_id: u64,
    max_accounts_scan: u64,
    chunk_size: usize,
) -> Result<Vec<LedgerEntryQuery>> {
    let window = max_accounts_scan.max(1).min(max_account_id);
    let start = {
        let cursor = USER_SCAN_CURSOR.load(Ordering::Relaxed);
        if cursor < 1 || cursor > max_account_id {
            1
        } else {
            cursor
        }
    };
    // Wrap so successive ticks cover every id in ceil(nonce / window) rounds.
    let ids: Vec<u64> = (0..window)
        .map(|offset| (start - 1 + offset) % max_account_id + 1)
        .collect();
    let next = (start - 1 + window) % max_account_id + 1;
    USER_SCAN_CURSOR.store(next, Ordering::Relaxed);

    if max_account_id > window {
        info!(
            target: "keeper.discovery",
            max_account_id,
            max_accounts_scan,
            scanned_from = start,
            next_tick_from = next,
            "per-user scan window rotating; full coverage every {} ticks",
            max_account_id.div_ceil(window)
        );
    }

    let mut rows: Vec<LedgerEntryQuery> = Vec::new();
    let chunk = chunk_size.max(1);

    for id_chunk in ids.chunks(chunk) {
        let mut keys = Vec::with_capacity(id_chunk.len() * 5);
        for &id in id_chunk {
            keys.push(ControllerUserKey::AccountMeta(id).to_ledger_key(controller_id)?);
            keys.push(ControllerUserKey::SupplyPositions(id).to_ledger_key(controller_id)?);
            keys.push(ControllerUserKey::BorrowPositions(id).to_ledger_key(controller_id)?);
            keys.push(ControllerUserKey::Delegates(id).to_ledger_key(controller_id)?);
            // The NFT `Owner` entry carries a shorter TTL than the controller's
            // account keys, so it archives first unless it is renewed too.
            if let (Some(nft_id), Ok(token_id)) = (position_nft_id.as_ref(), u32::try_from(id)) {
                keys.push(PositionNftUserKey::Owner(token_id).to_ledger_key(nft_id)?);
            }
        }
        rows.extend(client.get_ledger_entries(&keys).await?);
    }

    debug!(
        target: "keeper.discovery",
        scanned = ids.len(),
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

/// Which contract instances this tick keeps alive, and where each one's row
/// lands in the reply.
///
/// `get_ledger_entries` returns one row per input key in input order, so a row
/// is addressed by the index recorded when its key was pushed. The indices are
/// returned rather than recomputed at the call site, so adding a contract here
/// cannot silently repoint an existing lookup.
struct InstancePlan {
    keys: Vec<LedgerKey>,
    pool_row: Option<usize>,
    flash_row: Option<usize>,
    nft_row: Option<usize>,
}

/// Builds the instance-key plan. The position NFT is included because it holds
/// account ownership: its instance and code must outlive the `Owner` rows this
/// snapshot renews, since an archived NFT contract fails every controller
/// ownership check even while the `Owner` entries are still live.
fn plan_instance_keys(
    controller_id: &[u8; 32],
    pool_id: Option<&[u8; 32]>,
    flash_receiver: Option<&[u8; 32]>,
    position_nft_id: Option<&[u8; 32]>,
) -> InstancePlan {
    let mut keys = Vec::with_capacity(4);
    keys.push(contract_instance_key(controller_id));
    let pool_row = pool_id.map(|pool| {
        keys.push(contract_instance_key(pool));
        keys.len() - 1
    });
    let flash_row = flash_receiver.map(|receiver| {
        keys.push(contract_instance_key(receiver));
        keys.len() - 1
    });
    let nft_row = position_nft_id.map(|nft| {
        keys.push(contract_instance_key(nft));
        keys.len() - 1
    });
    InstancePlan {
        keys,
        pool_row,
        flash_row,
        nft_row,
    }
}

/// Maps an instance's executable to the Wasm hash whose `ContractCode` entry the
/// keeper must keep alive.
///
/// A `StellarAsset` executable has no code entry, so `None` is simply correct. A
/// CAP-83 `ExternalRef` has no code hash either — the executable hangs off
/// `executable_owner` rather than a hash-keyed entry — but unlike a SAC it means a
/// contract we are responsible for now uses a TTL model this keeper does not cover.
/// Return `None` so the remaining entries still get extended, and warn so an
/// operator learns about it before the entry expires rather than after.
fn wasm_hash_from_executable(executable: &ContractExecutable) -> Option<[u8; 32]> {
    match executable {
        ContractExecutable::Wasm(Hash(bytes)) => Some(*bytes),
        ContractExecutable::StellarAsset => None,
        ContractExecutable::ExternalRef(external) => {
            warn!(
                target: "keeper.discovery",
                owner = ?external.executable_owner,
                tag = ?external.tag,
                "contract executable is a CAP-83 external ref: no ContractCode entry \
                 exists to extend, so this contract's code TTL is outside keeper coverage"
            );
            None
        }
    }
}

/// Borrows the contract instance out of a ledger-entry row, or `None` when the
/// entry is absent or archived.
fn instance_from_row(row: &LedgerEntryQuery) -> Option<&ScContractInstance> {
    let LedgerEntryData::ContractData(cd) = row.value.as_ref()? else {
        return None;
    };
    match &cd.val {
        ScVal::ContractInstance(inst) => Some(inst),
        _ => None,
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

/// Converts the position-NFT sequential counter into the largest account id
/// that can exist.
///
/// The counter holds the NEXT free token id, and the NFT constructor consumes
/// id 0 as the controller's "new account" sentinel, so the largest usable id is
/// one below the counter. Saturates so an unset counter reports no accounts
/// rather than underflowing.
fn max_account_id_from_counter(next_token_id: u32) -> u64 {
    u64::from(next_token_id.saturating_sub(1))
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
    lookup_instance_scalar(instance, key.variant_name(), extract)
}

/// Decodes the price aggregator's `OracleKeys` index into the `PriceKey` set it
/// stores. Returns `None` when the index is absent so the caller can fall back
/// rather than treat "no index" as "no oracles". A malformed entry is an error:
/// silently returning an empty set would drop every oracle row from renewal.
fn lookup_oracle_keys(instance: &ScContractInstance) -> Result<Option<Vec<AggregatorPriceKey>>> {
    let raw = lookup_instance_scalar(
        instance,
        PriceAggregatorInstanceKey::OracleKeys.variant_name(),
        |v| Some(v.clone()),
    )?;
    let Some(ScVal::Vec(Some(items))) = raw else {
        return Ok(None);
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items.iter() {
        out.push(decode_price_key(item)?);
    }
    Ok(Some(out))
}

/// Decodes one `PriceKey` enum value: `Token(Address)` or `Ref(Symbol)`.
fn decode_price_key(value: &ScVal) -> Result<AggregatorPriceKey> {
    let ScVal::Vec(Some(parts)) = value else {
        return Err(anyhow!("PriceKey is not an enum vec"));
    };
    let [ScVal::Symbol(variant), payload] = &parts[..] else {
        return Err(anyhow!("PriceKey has an unexpected shape"));
    };
    match variant.to_string().as_str() {
        "Token" => {
            let ScVal::Address(ScAddress::Contract(ContractId(Hash(id)))) = payload else {
                return Err(anyhow!("PriceKey::Token payload is not a contract address"));
            };
            Ok(AggregatorPriceKey::Token(*id))
        }
        "Ref" => {
            let ScVal::Symbol(name) = payload else {
                return Err(anyhow!("PriceKey::Ref payload is not a symbol"));
            };
            Ok(AggregatorPriceKey::Ref(name.to_string()))
        }
        other => Err(anyhow!("unknown PriceKey variant {other}")),
    }
}

/// Reads a unit-variant instance-storage entry by its variant name.
fn lookup_instance_scalar<T>(
    instance: &ScContractInstance,
    variant_name: &str,
    extract: impl Fn(&ScVal) -> Option<T>,
) -> Result<Option<T>> {
    let needle = needle_for(variant_name)?;
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

fn needle_for(variant_name: &str) -> Result<ScVal> {
    let symbol =
        ScSymbol(StringM::<32>::try_from(variant_name).map_err(|_| anyhow!("symbol too long"))?);
    Ok(ScVal::Vec(Some(stellar_xdr::ScVec(
        vec![ScVal::Symbol(symbol)]
            .try_into()
            .map_err(|_| anyhow!("vec convert"))?,
    ))))
}

pub fn self_check(contracts: &ContractsConfig) -> Result<Vec<HubAssetKey>> {
    configured_market_assets(contracts)
}

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
        .simulate(&envelope, Some(AuthMode::Enforce))
        .await
        .context("simulate update_indexes(empty) for boot preflight")?;

    if let Some(err) = sim.error {
        return Err(anyhow!(
            "update_indexes simulation failed with `{err}` for signer {caller_strkey}."
        ));
    }
    Ok(())
}

const SIM_FEE_STROOPS: u32 = 100;

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::{ContractDataDurability, ScVec};

    /// Independently reconstructs the `#[contracttype]` encoding of a
    /// single-argument enum variant, so the test does not just re-run the
    /// production encoder.
    fn sc_enum_for_test(variant: &str, arg: u32) -> ScVal {
        ScVal::Vec(Some(stellar_xdr::ScVec(
            vec![
                ScVal::Symbol(ScSymbol(StringM::<32>::try_from(variant).unwrap())),
                ScVal::U32(arg),
            ]
            .try_into()
            .unwrap(),
        )))
    }

    /// Builds a contract instance whose storage holds one unit-variant key.
    fn instance_with(variant: &str, val: ScVal) -> ScContractInstance {
        let entry = ScMapEntry {
            key: needle_for(variant).unwrap(),
            val,
        };
        ScContractInstance {
            executable: stellar_xdr::ContractExecutable::Wasm(Hash([0u8; 32])),
            storage: Some(stellar_xdr::ScMap(vec![entry].try_into().unwrap())),
        }
    }

    fn price_key_token(id: [u8; 32]) -> ScVal {
        ScVal::Vec(Some(ScVec(
            vec![
                ScVal::Symbol("Token".try_into().unwrap()),
                ScVal::Address(ScAddress::Contract(ContractId(Hash(id)))),
            ]
            .try_into()
            .unwrap(),
        )))
    }

    fn price_key_ref(name: &str) -> ScVal {
        ScVal::Vec(Some(ScVec(
            vec![
                ScVal::Symbol("Ref".try_into().unwrap()),
                ScVal::Symbol(name.try_into().unwrap()),
            ]
            .try_into()
            .unwrap(),
        )))
    }

    /// `Ref` rows are the reason the index is read at all: no list of market
    /// addresses can produce them, and `Ref("BTC")` backs the SolvBTC assets.
    #[test]
    fn oracle_keys_index_decodes_token_and_ref_rows() {
        let instance = instance_with(
            "OracleKeys",
            ScVal::Vec(Some(ScVec(
                vec![price_key_token([9u8; 32]), price_key_ref("BTC")]
                    .try_into()
                    .unwrap(),
            ))),
        );

        let keys = lookup_oracle_keys(&instance).unwrap().unwrap();

        assert_eq!(
            keys,
            vec![
                AggregatorPriceKey::Token([9u8; 32]),
                AggregatorPriceKey::Ref("BTC".to_string()),
            ]
        );
    }

    /// A missing index must read as "unknown" so the caller falls back to the
    /// configured markets. Returning an empty set here would silently drop every
    /// oracle row from renewal — the exact failure this rewrite fixes.
    #[test]
    fn a_missing_oracle_keys_index_is_none_not_empty() {
        let instance = instance_with("SomethingElse", ScVal::U32(1));
        assert!(lookup_oracle_keys(&instance).unwrap().is_none());
    }

    /// A malformed index must fail loudly rather than silently renew a subset.
    #[test]
    fn a_malformed_oracle_keys_index_is_an_error() {
        let instance = instance_with(
            "OracleKeys",
            ScVal::Vec(Some(ScVec(vec![ScVal::U32(7)].try_into().unwrap()))),
        );
        assert!(lookup_oracle_keys(&instance).is_err());
    }

    /// An absent or archived row must not resolve to a counter. The tick has to
    /// survive it: aborting here would stop discovery before a restore for the
    /// NFT instance could be planned, so the contract could never come back.
    #[test]
    fn an_archived_nft_instance_yields_no_counter() {
        let absent = LedgerEntryQuery {
            key: contract_instance_key(&[4u8; 32]),
            value: None,
            live_until_ledger: None,
        };
        assert!(instance_from_row(&absent).is_none());
        // The snapshot maps that to "no accounts scanned this tick", not a panic.
        assert_eq!(max_account_id_from_counter(0), 0);
    }

    #[test]
    fn the_position_nft_instance_is_kept_alive() {
        let ctrl = [1u8; 32];
        let pool = [2u8; 32];
        let flash = [3u8; 32];
        let nft = [4u8; 32];
        let plan = plan_instance_keys(&ctrl, Some(&pool), Some(&flash), Some(&nft));
        assert!(
            plan.keys.contains(&contract_instance_key(&nft)),
            "the NFT holds account ownership; if its instance can archive, \
             renewing the Owner rows alone does not keep accounts usable"
        );
        // Each tracked row must address its own contract, not a neighbour's.
        assert_eq!(plan.keys[0], contract_instance_key(&ctrl));
        assert_eq!(
            plan.keys[plan.pool_row.unwrap()],
            contract_instance_key(&pool)
        );
        assert_eq!(
            plan.keys[plan.flash_row.unwrap()],
            contract_instance_key(&flash)
        );
        assert_eq!(
            plan.keys[plan.nft_row.unwrap()],
            contract_instance_key(&nft)
        );
    }

    #[test]
    fn instance_rows_stay_addressable_when_optional_contracts_are_absent() {
        let ctrl = [1u8; 32];
        let flash = [3u8; 32];
        let plan = plan_instance_keys(&ctrl, None, Some(&flash), None);
        assert_eq!(plan.pool_row, None);
        assert_eq!(plan.nft_row, None);
        assert_eq!(
            plan.keys[plan.flash_row.unwrap()],
            contract_instance_key(&flash),
            "dropping the pool must not shift the flash-receiver row"
        );
        assert_eq!(plan.keys.len(), 2);
    }

    #[test]
    fn nft_owner_key_targets_the_nft_contract_not_the_controller() {
        let nft_id = [7u8; 32];
        let controller_id = [9u8; 32];
        let key = PositionNftUserKey::Owner(3).to_ledger_key(&nft_id).unwrap();
        let LedgerKey::ContractData(cd) = &key else {
            panic!("expected contract data key");
        };
        assert_eq!(
            cd.contract,
            ScAddress::Contract(ContractId(Hash(nft_id))),
            "Owner lives in the NFT contract; renewing it against the controller is a no-op"
        );
        assert_ne!(
            cd.contract,
            ScAddress::Contract(ContractId(Hash(controller_id)))
        );
        assert_eq!(cd.durability, ContractDataDurability::Persistent);
        assert_eq!(cd.key, sc_enum_for_test("Owner", 3));
    }

    #[test]
    fn reads_the_position_nft_sequential_counter() {
        let instance = instance_with("TokenIdCounter", ScVal::U32(7));
        let got = lookup_instance_scalar(
            &instance,
            PositionNftInstanceKey::TokenIdCounter.variant_name(),
            scval_u32,
        )
        .unwrap();
        assert_eq!(got, Some(7));
    }

    #[test]
    fn counter_miss_does_not_resolve_to_another_key() {
        let instance = instance_with("LastHubId", ScVal::U32(7));
        let got = lookup_instance_scalar(
            &instance,
            PositionNftInstanceKey::TokenIdCounter.variant_name(),
            scval_u32,
        )
        .unwrap();
        assert_eq!(
            got, None,
            "a different variant must not satisfy the counter lookup"
        );
    }

    /// The sequential counter holds the NEXT free token id, and the NFT
    /// constructor consumes id 0 as the controller's "new account" sentinel.
    /// So the largest usable account id is `counter - 1`, and a protocol with
    /// no accounts yet reports 0 rather than underflowing.
    #[test]
    fn max_account_id_is_one_below_the_next_free_token_id() {
        let max = max_account_id_from_counter;
        assert_eq!(max(0), 0, "unset counter must not underflow");
        assert_eq!(
            max(1),
            0,
            "only the burned sentinel exists: no accounts yet"
        );
        assert_eq!(max(2), 1, "one account minted, id 1");
        assert_eq!(max(7), 6);
        assert_eq!(max(u32::MAX), u64::from(u32::MAX - 1));
    }

    #[test]
    fn resolve_accepts_testnet_governance_address() {
        let contracts = ContractsConfig {
            controller: "CBSCWXCIAASFR2F2332D2I7C6VWUJZKUW4ONOZR2LZ32KOZ5UZVNJ3LA".into(),
            pool_wasm_hash: "a1e7db9b32626c8d4c57343c50407956ea1b642054bf6aee0a613da06359a6fa"
                .into(),
            flash_loan_receiver: Some(
                "CCYDZ6SLHGZKBJF3MNKRK2QPITSVTHL5NYWKWWPMNSOTW4HHCK32JNLZ".into(),
            ),
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
            flash_loan_receiver: Some(
                "CCYDZ6SLHGZKBJF3MNKRK2QPITSVTHL5NYWKWWPMNSOTW4HHCK32JNLZ".into(),
            ),
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
