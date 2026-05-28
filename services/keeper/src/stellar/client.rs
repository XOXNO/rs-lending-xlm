//! Thin wrapper around `stellar_rpc_client::Client` so the rest of the keeper
//! can mock it in tests if needed and so we can layer keeper-specific
//! conveniences (chunked `get_ledger_entries`, contract-id parsing).

use anyhow::{anyhow, Context, Result};
use std::time::Duration;
use stellar_rpc_client::Client as InnerClient;
use stellar_xdr::curr::{
    AccountId, Hash, LedgerEntryData, LedgerKey, PublicKey, ScContractInstance, Uint256,
};

use crate::config::RpcConfig;

pub struct RpcClient {
    inner: InnerClient,
    base_url: String,
}

impl RpcClient {
    pub fn new(cfg: &RpcConfig) -> Result<Self> {
        let _ = Duration::from_secs(cfg.timeout_seconds);
        let inner =
            InnerClient::new(&cfg.url).with_context(|| format!("connect RPC at {}", cfg.url))?;
        Ok(Self {
            inner,
            base_url: cfg.url.clone(),
        })
    }

    pub fn inner(&self) -> &InnerClient {
        &self.inner
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn latest_ledger(&self) -> Result<u32> {
        let resp = self
            .inner
            .get_latest_ledger()
            .await
            .context("get_latest_ledger")?;
        Ok(resp.sequence)
    }

    /// Read the controller's instance entry (which carries `AccountNonce`,
    /// `PoolTemplate`, etc.) by contract id.
    pub async fn get_contract_instance(
        &self,
        contract_id: &[u8; 32],
    ) -> Result<ScContractInstance> {
        let strkey = format!("{}", stellar_strkey::Contract(*contract_id));
        self.inner
            .get_contract_instance(contract_id)
            .await
            .with_context(|| format!("get_contract_instance({strkey})"))
    }

    /// Batched ledger-key lookup. Returns one tuple per requested key. Missing
    /// entries surface as `None` for `live_until`.
    pub async fn get_ledger_entries(
        &self,
        keys: &[LedgerKey],
    ) -> Result<Vec<LedgerEntryQuery>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let resp = self
            .inner
            .get_full_ledger_entries(keys)
            .await
            .context("get_full_ledger_entries")?;

        // The RPC returns entries in input order, with absent entries omitted.
        // We zip on requested-vs-returned by index, padding `None` for missing.
        let mut found = std::collections::HashMap::<Vec<u8>, _>::with_capacity(resp.entries.len());
        for entry in resp.entries.iter() {
            let key_xdr = xdr_bytes(&entry.key)?;
            found.insert(key_xdr, entry);
        }

        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            let key_xdr = xdr_bytes(k)?;
            let row = match found.get(&key_xdr) {
                Some(entry) => LedgerEntryQuery {
                    key: k.clone(),
                    value: Some(entry.val.clone()),
                    live_until_ledger: entry.live_until_ledger_seq,
                },
                None => LedgerEntryQuery {
                    key: k.clone(),
                    value: None,
                    live_until_ledger: None,
                },
            };
            out.push(row);
        }
        Ok(out)
    }

    /// Resolve a stellar account id (G… strkey) into a sequence number.
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

/// Decode an account strkey (`G...`) into the XDR `AccountId` wrapper.
pub fn account_id_from_strkey(g_strkey: &str) -> Result<AccountId> {
    let pk = stellar_strkey::ed25519::PublicKey::from_string(g_strkey)
        .map_err(|e| anyhow!("invalid G... account id {g_strkey}: {e}"))?;
    Ok(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(pk.0))))
}

/// Decode a contract strkey (`C...`) into the raw 32-byte contract id.
pub fn contract_id_from_strkey(c_strkey: &str) -> Result<[u8; 32]> {
    let c = stellar_strkey::Contract::from_string(c_strkey)
        .map_err(|e| anyhow!("invalid C... contract id {c_strkey}: {e}"))?;
    Ok(c.0)
}

/// Decode a 32-byte hex string into a contract / wasm hash.
pub fn hash32_from_hex(hex_str: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(hex_str.trim())
        .map_err(|e| anyhow!("invalid 32-byte hex {hex_str}: {e}"))?;
    if bytes.len() != 32 {
        return Err(anyhow!(
            "expected 32 bytes, got {} from {hex_str}",
            bytes.len()
        ));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

#[allow(dead_code)]
pub fn hash_from_bytes(bytes: [u8; 32]) -> Hash {
    Hash(bytes)
}

fn xdr_bytes<T: stellar_xdr::curr::WriteXdr>(value: &T) -> Result<Vec<u8>> {
    use stellar_xdr::curr::Limits;
    value
        .to_xdr(Limits::none())
        .map_err(|e| anyhow!("xdr encode: {e}"))
}
