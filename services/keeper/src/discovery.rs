//! Read controller + pool storage, build the lists of work the scheduler
//! turns into transactions.

use anyhow::{anyhow, Context, Result};
use stellar_xdr::curr::{Hash, LedgerKey, ScMapEntry, ScSymbol, ScVal, StringM};
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
    /// Persistent ledger entries we may want to bump. Each row carries its
    /// `live_until` and the decoded key.
    pub persistent_entries: Vec<LedgerEntryQuery>,
    /// Wasm code entries (no per-key TTL filter at the discovery layer; the
    /// scheduler decides whether to extend based on policy).
    pub wasm_code_entries: Vec<LedgerEntryQuery>,
    /// Account-id ceiling (read from instance storage); the keeper will
    /// chunk-read 1..=nonce per tick.
    pub account_nonce: u64,
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
    debug!(target: "keeper.discovery", account_nonce, "instance read");

    // -- Pool list (persistent) --
    let pool_list_key = ControllerPersistentKey::PoolsList.to_ledger_key(&controller_id)?;
    let mut persistent_entries = client.get_ledger_entries(&[pool_list_key]).await?;
    let assets = extract_pools_list(&persistent_entries).unwrap_or_default();

    // -- Per-asset persistent state --
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
        persistent_entries.extend(rows);
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

    // -- Wasm code entries (controller + pool template + flash receiver) --
    let mut wasm_keys: Vec<LedgerKey> = Vec::new();
    wasm_keys.push(contract_code_key(&pool_wasm_hash));
    if let Some(ctrl_hash) = controller_wasm_hash {
        wasm_keys.push(contract_code_key(&ctrl_hash));
    } else {
        warn!(target: "keeper.discovery", "controller wasm hash unresolved — pool template extend only");
    }
    // Resolve flash receiver wasm hash via its own instance entry.
    let flash_instance_key = contract_instance_key(&flash_receiver_id);
    let flash_rows = client
        .get_ledger_entries(std::slice::from_ref(&flash_instance_key))
        .await?;
    if let Some(flash_hash) = wasm_hash_from_instance_row(flash_rows.first()) {
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
        persistent_entries,
        wasm_code_entries,
        account_nonce,
    })
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

fn wasm_hash_from_instance_row(row: Option<&LedgerEntryQuery>) -> Option<[u8; 32]> {
    use stellar_xdr::curr::{ContractExecutable, LedgerEntryData, ScVal};
    let row = row?;
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
    let symbol_text = key.variant_name();
    let symbol = ScSymbol(StringM::<32>::try_from(symbol_text).map_err(|_| anyhow!("symbol too long"))?);
    let needle = ScVal::Vec(Some(stellar_xdr::curr::ScVec(
        vec![ScVal::Symbol(symbol)]
            .try_into()
            .map_err(|_| anyhow!("vec convert"))?,
    )));
    let storage = match &instance.storage {
        Some(m) => m,
        None => return Ok(None),
    };
    for ScMapEntry { key, val } in storage.0.iter() {
        if key == &needle {
            if let ScVal::U64(v) = val {
                return Ok(Some(*v));
            }
        }
    }
    Ok(None)
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

/// Boot-time auth gate: simulate `keepalive_pools(caller, empty_vec)` and
/// refuse to start unless simulation succeeds. A successful simulation
/// confirms the KEEPER role is granted to the signer.
pub async fn assert_keeper_role(
    client: &RpcClient,
    controller_strkey: &str,
    caller_strkey: &str,
) -> Result<()> {
    use crate::stellar::invoke::keepalive_pools;
    use stellar_xdr::curr::{
        Memo, MuxedAccount, Preconditions, SequenceNumber, Transaction, TransactionEnvelope,
        TransactionExt, TransactionV1Envelope, Uint256, VecM,
    };

    let controller_id = contract_id_from_strkey(controller_strkey)?;
    let job = keepalive_pools(&controller_id, caller_strkey, &[])?;

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
