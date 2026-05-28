//! Read controller + pool storage, build the lists of work the scheduler
//! turns into transactions.

use anyhow::{anyhow, Context, Result};
use stellar_xdr::curr::{
    ContractId, Hash, LedgerEntryData, LedgerKey, ScAddress, ScMapEntry, ScSymbol, ScVal,
    StringM,
};
use tracing::{debug, info, warn};

use crate::keys::{
    contract_code_key, contract_instance_key, ControllerInstanceKey, ControllerPersistentKey,
};
use crate::stellar::client::{
    account_id_from_strkey, contract_id_from_strkey, hash32_from_hex, LedgerEntryQuery, RpcClient,
};

/// One snapshot of "what needs bumping" assembled by a single tick.
#[derive(Debug, Default)]
pub struct DiscoverySnapshot {
    pub current_ledger: u32,
    pub controller_id: [u8; 32],
    pub pool_wasm_hash: [u8; 32],
    pub flash_receiver_id: [u8; 32],
    pub controller_wasm_hash: Option<[u8; 32]>,
    pub assets: Vec<[u8; 32]>,
    pub pool_contract_ids: Vec<[u8; 32]>,
    /// Persistent ledger entries we may want to bump. Each row carries its
    /// `live_until` and the decoded key.
    pub persistent_entries: Vec<LedgerEntryQuery>,
    /// Contract-instance entries (controller + pools + flash receiver).
    pub instance_entries: Vec<LedgerEntryQuery>,
    /// Wasm code entries (no per-key TTL filter at the discovery layer; the
    /// scheduler decides whether to extend based on policy).
    pub wasm_code_entries: Vec<LedgerEntryQuery>,
    /// Account-id ceiling (read from instance storage); the keeper will
    /// chunk-read 1..=nonce per tick.
    pub account_nonce: u64,
    /// E-mode category ceiling (read from instance storage).
    pub last_emode_category_id: u32,
}

pub struct DiscoveryConfig {
    pub controller_strkey: String,
    pub pool_wasm_hash_hex: String,
    pub flash_receiver_strkey: String,
    pub account_chunk: usize,
    pub asset_chunk: usize,
    pub include_account_sweep: bool,
}

pub async fn snapshot(
    client: &RpcClient,
    cfg: &DiscoveryConfig,
) -> Result<DiscoverySnapshot> {
    let controller_id = contract_id_from_strkey(&cfg.controller_strkey)?;
    let pool_wasm_hash = hash32_from_hex(&cfg.pool_wasm_hash_hex)?;
    let flash_receiver_id = contract_id_from_strkey(&cfg.flash_receiver_strkey)?;

    let current_ledger = client.latest_ledger().await?;
    info!(
        target: "keeper.discovery",
        controller = %cfg.controller_strkey,
        current_ledger,
        "tick start"
    );

    // -- Controller instance: PoolsList + AccountNonce + controller wasm hash --
    let instance = client.get_contract_instance(&controller_id).await?;
    let controller_wasm_hash = controller_wasm_from_instance(&instance);
    let account_nonce = lookup_u64(&instance, ControllerInstanceKey::AccountNonce)?.unwrap_or(0);
    let last_emode_category_id =
        lookup_u32(&instance, ControllerInstanceKey::LastEModeCategoryId)?.unwrap_or(0);
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

    // -- Per-asset persistent state --
    let mut market_rows: Vec<LedgerEntryQuery> = Vec::new();
    for chunk in assets.chunks(cfg.asset_chunk.max(1)) {
        let mut keys = Vec::with_capacity(chunk.len() * 2);
        for asset in chunk {
            keys.push(ControllerPersistentKey::Market(*asset).to_ledger_key(&controller_id)?);
            keys.push(
                ControllerPersistentKey::IsolatedDebt(*asset)
                    .to_ledger_key(&controller_id)?,
            );
        }
        let rows = client.get_ledger_entries(&keys).await?;
        // Stash market rows for pool_address extraction; pass IsolatedDebt
        // straight through.
        for row in rows {
            if matches!(decode_market_key_name(&row.key), Some(s) if s == "Market") {
                market_rows.push(row);
            } else {
                persistent_entries.push(row);
            }
        }
    }

    // -- Resolve pool contract ids by parsing each Market entry's
    // `pool_address` field; also append the Market rows back for TTL bumping.
    let mut pool_contract_ids: Vec<[u8; 32]> = Vec::new();
    for row in &market_rows {
        if let Some(pool_id) = extract_pool_address_from_market(row) {
            pool_contract_ids.push(pool_id);
        }
    }
    persistent_entries.extend(market_rows);

    // -- E-mode category sweep --
    if last_emode_category_id > 0 {
        let chunk_size = cfg.asset_chunk.max(1);
        for chunk in (1..=last_emode_category_id)
            .collect::<Vec<_>>()
            .chunks(chunk_size)
        {
            let keys: Result<Vec<LedgerKey>> = chunk
                .iter()
                .map(|id| ControllerPersistentKey::EModeCategory(*id).to_ledger_key(&controller_id))
                .collect();
            let rows = client.get_ledger_entries(&keys?).await?;
            persistent_entries.extend(rows);
        }
    }

    // -- User account triplets --
    if cfg.include_account_sweep && account_nonce > 0 {
        let chunk_size = cfg.account_chunk.max(1);
        let total_keys = (account_nonce as usize).saturating_mul(3);
        debug!(target: "keeper.discovery", total_keys, "starting account sweep");
        let mut id_buffer = Vec::with_capacity(chunk_size);
        for id in 1..=account_nonce {
            id_buffer.push(id);
            if id_buffer.len() == chunk_size {
                let keys = account_keys_for_chunk(&controller_id, &id_buffer)?;
                let rows = client.get_ledger_entries(&keys).await?;
                persistent_entries.extend(rows);
                id_buffer.clear();
            }
        }
        if !id_buffer.is_empty() {
            let keys = account_keys_for_chunk(&controller_id, &id_buffer)?;
            let rows = client.get_ledger_entries(&keys).await?;
            persistent_entries.extend(rows);
        }
    }

    // -- Instance entries (controller + each pool + flash receiver) --
    let mut instance_keys = vec![contract_instance_key(&controller_id)];
    for pool_id in &pool_contract_ids {
        instance_keys.push(contract_instance_key(pool_id));
    }
    instance_keys.push(contract_instance_key(&flash_receiver_id));
    let instance_entries = client.get_ledger_entries(&instance_keys).await?;

    // -- Wasm code entries (controller + pool template + flash receiver) --
    let mut wasm_keys: Vec<LedgerKey> = vec![contract_code_key(&pool_wasm_hash)];
    if let Some(ctrl_hash) = controller_wasm_hash {
        wasm_keys.push(contract_code_key(&ctrl_hash));
    } else {
        warn!(target: "keeper.discovery", "controller wasm hash unresolved — pool template extend only");
    }
    // Pull flash-receiver wasm hash from its instance entry we just read.
    if let Some(flash_hash) = instance_entries
        .last()
        .and_then(wasm_hash_from_instance_row)
    {
        wasm_keys.push(contract_code_key(&flash_hash));
    }
    let wasm_code_entries = client.get_ledger_entries(&wasm_keys).await?;

    Ok(DiscoverySnapshot {
        current_ledger,
        controller_id,
        pool_wasm_hash,
        flash_receiver_id,
        controller_wasm_hash,
        assets,
        pool_contract_ids,
        persistent_entries,
        instance_entries,
        wasm_code_entries,
        account_nonce,
        last_emode_category_id,
    })
}

fn decode_market_key_name(key: &LedgerKey) -> Option<String> {
    let LedgerKey::ContractData(cd) = key else {
        return None;
    };
    let ScVal::Vec(Some(v)) = &cd.key else { return None };
    let first = v.0.first()?;
    let ScVal::Symbol(ScSymbol(sym)) = first else { return None };
    Some(sym.to_utf8_string_lossy())
}

/// Walk a Market entry's ScVal::Map and pull out the `pool_address` field.
fn extract_pool_address_from_market(row: &LedgerEntryQuery) -> Option<[u8; 32]> {
    let LedgerEntryData::ContractData(cd) = row.value.as_ref()? else {
        return None;
    };
    let ScVal::Map(Some(map)) = &cd.val else { return None };
    for ScMapEntry { key, val } in map.0.iter() {
        let ScVal::Symbol(ScSymbol(sym)) = key else { continue };
        if sym.to_utf8_string_lossy() == "pool_address" {
            if let ScVal::Address(ScAddress::Contract(ContractId(Hash(bytes)))) = val {
                return Some(*bytes);
            }
        }
    }
    None
}

fn account_keys_for_chunk(controller_id: &[u8; 32], ids: &[u64]) -> Result<Vec<LedgerKey>> {
    let mut keys = Vec::with_capacity(ids.len() * 3);
    for id in ids {
        keys.push(ControllerPersistentKey::AccountMeta(*id).to_ledger_key(controller_id)?);
        keys.push(ControllerPersistentKey::SupplyPositions(*id).to_ledger_key(controller_id)?);
        keys.push(ControllerPersistentKey::BorrowPositions(*id).to_ledger_key(controller_id)?);
    }
    Ok(keys)
}

fn controller_wasm_from_instance(instance: &stellar_xdr::curr::ScContractInstance) -> Option<[u8; 32]> {
    use stellar_xdr::curr::ContractExecutable;
    match &instance.executable {
        ContractExecutable::Wasm(Hash(bytes)) => Some(*bytes),
        ContractExecutable::StellarAsset => None,
    }
}

fn wasm_hash_from_instance_row(row: &LedgerEntryQuery) -> Option<[u8; 32]> {
    use stellar_xdr::curr::{ContractExecutable, LedgerEntryData, ScVal};
    let LedgerEntryData::ContractData(cd) = row.value.as_ref()? else {
        return None;
    };
    if let ScVal::ContractInstance(inst) = &cd.val {
        if let ContractExecutable::Wasm(Hash(bytes)) = inst.executable {
            return Some(bytes);
        }
    }
    None
}

fn lookup_u64(
    instance: &stellar_xdr::curr::ScContractInstance,
    key: ControllerInstanceKey,
) -> Result<Option<u64>> {
    let needle = needle_for(key)?;
    let Some(storage) = &instance.storage else { return Ok(None) };
    for ScMapEntry { key, val } in storage.0.iter() {
        if key == &needle {
            if let ScVal::U64(v) = val {
                return Ok(Some(*v));
            }
        }
    }
    Ok(None)
}

fn lookup_u32(
    instance: &stellar_xdr::curr::ScContractInstance,
    key: ControllerInstanceKey,
) -> Result<Option<u32>> {
    let needle = needle_for(key)?;
    let Some(storage) = &instance.storage else { return Ok(None) };
    for ScMapEntry { key, val } in storage.0.iter() {
        if key == &needle {
            if let ScVal::U32(v) = val {
                return Ok(Some(*v));
            }
        }
    }
    Ok(None)
}

fn needle_for(key: ControllerInstanceKey) -> Result<ScVal> {
    let symbol = ScSymbol(
        StringM::<32>::try_from(key.variant_name())
            .map_err(|_| anyhow!("symbol too long"))?,
    );
    Ok(ScVal::Vec(Some(stellar_xdr::curr::ScVec(
        vec![ScVal::Symbol(symbol)]
            .try_into()
            .map_err(|_| anyhow!("vec convert"))?,
    ))))
}

fn extract_pools_list(rows: &[LedgerEntryQuery]) -> Option<Vec<[u8; 32]>> {
    use stellar_xdr::curr::{ContractId, LedgerEntryData, ScAddress, ScVal};
    let row = rows.first()?;
    let LedgerEntryData::ContractData(cd) = row.value.as_ref()? else {
        return None;
    };
    let ScVal::Vec(Some(vec)) = &cd.val else { return None };
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
        // Either the controller is fresh (empty list) or our encoding is
        // wrong — soroban-sdk's behavior is that `Vec::new` is still stored,
        // so a *missing* entry is suspicious but not definitively wrong.
        warn!(target: "keeper.discovery", "PoolsList missing from ledger — controller may be fresh");
        return Ok(Vec::new());
    }
    Ok(extract_pools_list(rows.as_slice()).unwrap_or_default())
}

/// Boot-time auth gate: simulate `update_indexes(caller, empty_vec)` and
/// refuse to start unless simulation succeeds. A successful simulation
/// confirms the KEEPER role is granted to the signer. Called only when
/// the operator has enabled the index-refresh loop — pure-TTL keepers
/// skip this since `ExtendFootprintTtl` is permissionless.
pub async fn assert_keeper_role(
    client: &RpcClient,
    controller_strkey: &str,
    caller_strkey: &str,
) -> Result<()> {
    use crate::stellar::invoke::update_indexes;
    use stellar_xdr::curr::{
        Memo, MuxedAccount, Preconditions, SequenceNumber, Transaction, TransactionEnvelope,
        TransactionExt, TransactionV1Envelope, Uint256, VecM,
    };

    let controller_id = contract_id_from_strkey(controller_strkey)?;
    let job = update_indexes(&controller_id, caller_strkey, &[])?;

    let account_id = account_id_from_strkey(caller_strkey)?;
    let source_account = MuxedAccount::Ed25519(match account_id.0 {
        stellar_xdr::curr::PublicKey::PublicKeyTypeEd25519(Uint256(bytes)) => Uint256(bytes),
    });

    let ops: VecM<stellar_xdr::curr::Operation, 100> = vec![job.op]
        .try_into()
        .map_err(|_| anyhow!("op count overflow"))?;

    let tx = Transaction {
        source_account,
        fee: 100,
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
        .context("simulate keepalive_pools(empty) for KEEPER role check")?;

    if let Some(err) = sim.error {
        return Err(anyhow!(
            "KEEPER role check failed: simulation rejected with `{err}`. Grant role to {caller_strkey}."
        ));
    }
    Ok(())
}
