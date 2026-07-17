//! RPC client helpers used by the keeper.

use anyhow::{anyhow, Context, Result};
use std::collections::{HashMap, HashSet};
use stellar_rpc_client::Client as InnerClient;
use stellar_xdr::curr::{
    AccountId, LedgerEntryData, LedgerKey, MuxedAccount, PublicKey, ScContractInstance, Uint256,
};

use crate::config::RpcConfig;

pub struct RpcClient {
    inner: InnerClient,
}

impl RpcClient {
    pub fn new(cfg: &RpcConfig) -> Result<Self> {
        let inner =
            InnerClient::new(&cfg.url).with_context(|| format!("connect RPC at {}", cfg.url))?;
        Ok(Self { inner })
    }

    pub fn inner(&self) -> &InnerClient {
        &self.inner
    }

    pub async fn latest_ledger(&self) -> Result<u32> {
        let resp = self
            .inner
            .get_latest_ledger()
            .await
            .context("get_latest_ledger")?;
        Ok(resp.sequence)
    }

    pub async fn get_contract_instance(
        &self,
        contract_id: &[u8; 32],
    ) -> Result<ScContractInstance> {
        self.inner
            .get_contract_instance(contract_id)
            .await
            .with_context(|| {
                format!(
                    "get_contract_instance({})",
                    stellar_strkey::Contract(*contract_id)
                )
            })
    }

    /// Look up ledger keys; response order matches the request.
    pub async fn get_ledger_entries(&self, keys: &[LedgerKey]) -> Result<Vec<LedgerEntryQuery>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        // RPC rejects duplicate keys (cryptic captive-core 404). Dual-hub listings
        // repeat AssetOracle (asset-only key) across markets — dedupe request;
        // reassembly still emits one row per requested key.
        let mut seen = HashSet::with_capacity(keys.len());
        let unique: Vec<LedgerKey> = keys
            .iter()
            .filter(|k| seen.insert((*k).clone()))
            .cloned()
            .collect();
        let resp = self
            .inner
            .get_full_ledger_entries(&unique)
            .await
            .context("get_full_ledger_entries")?;

        // RPC omits absent entries; reassemble in request order.
        let mut found = HashMap::with_capacity(resp.entries.len());
        for entry in &resp.entries {
            found.insert(&entry.key, entry);
        }

        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            let (value, live_until_ledger) = match found.get(k) {
                Some(entry) => (Some(entry.val.clone()), entry.live_until_ledger_seq),
                None => (None, None),
            };
            out.push(LedgerEntryQuery {
                key: k.clone(),
                value,
                live_until_ledger,
            });
        }
        Ok(out)
    }

    pub async fn get_account_sequence(&self, account_strkey: &str) -> Result<i64> {
        let entry = self
            .inner
            .get_account(account_strkey)
            .await
            .with_context(|| format!("get_account({account_strkey})"))?;
        Ok(entry.seq_num.0)
    }
}

#[derive(Debug, Clone)]
pub struct LedgerEntryQuery {
    pub key: LedgerKey,
    pub value: Option<LedgerEntryData>,
    pub live_until_ledger: Option<u32>,
}

pub fn account_id_from_strkey(g_strkey: &str) -> Result<AccountId> {
    let pk = stellar_strkey::ed25519::PublicKey::from_string(g_strkey)
        .map_err(|e| anyhow!("invalid G... account id {g_strkey}: {e}"))?;
    Ok(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(pk.0))))
}

pub fn muxed_account_from_strkey(g_strkey: &str) -> Result<MuxedAccount> {
    let AccountId(PublicKey::PublicKeyTypeEd25519(key)) = account_id_from_strkey(g_strkey)?;
    Ok(MuxedAccount::Ed25519(key))
}

pub fn contract_id_from_strkey(c_strkey: &str) -> Result<[u8; 32]> {
    let c = stellar_strkey::Contract::from_string(c_strkey)
        .map_err(|e| anyhow!("invalid C... contract id {c_strkey}: {e}"))?;
    Ok(c.0)
}

pub fn hash32_from_hex(hex_str: &str) -> Result<[u8; 32]> {
    let bytes =
        hex::decode(hex_str.trim()).map_err(|e| anyhow!("invalid 32-byte hex {hex_str}: {e}"))?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| anyhow!("expected 32 bytes, got {} from {hex_str}", v.len()))
}
