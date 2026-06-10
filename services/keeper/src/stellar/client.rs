//! RPC client helpers used by the keeper.

use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
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

    /// Reads a contract instance entry by contract id.
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

    /// Looks up ledger keys and preserves request order.
    pub async fn get_ledger_entries(&self, keys: &[LedgerKey]) -> Result<Vec<LedgerEntryQuery>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let resp = self
            .inner
            .get_full_ledger_entries(keys)
            .await
            .context("get_full_ledger_entries")?;

        // The RPC omits absent entries, so index what came back by key and
        // reassemble in request order. `LedgerKey` is `Hash + Eq`, so it serves
        // as the map key directly — no XDR re-encoding per key.
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

    /// Resolves an account strkey to its sequence number.
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

/// Decode an account strkey (`G...`) into a `MuxedAccount` transaction source.
pub fn muxed_account_from_strkey(g_strkey: &str) -> Result<MuxedAccount> {
    let AccountId(PublicKey::PublicKeyTypeEd25519(key)) = account_id_from_strkey(g_strkey)?;
    Ok(MuxedAccount::Ed25519(key))
}

/// Decode a contract strkey (`C...`) into the raw 32-byte contract id.
pub fn contract_id_from_strkey(c_strkey: &str) -> Result<[u8; 32]> {
    let c = stellar_strkey::Contract::from_string(c_strkey)
        .map_err(|e| anyhow!("invalid C... contract id {c_strkey}: {e}"))?;
    Ok(c.0)
}

/// Decode a 32-byte hex string into a contract / wasm hash.
pub fn hash32_from_hex(hex_str: &str) -> Result<[u8; 32]> {
    let bytes =
        hex::decode(hex_str.trim()).map_err(|e| anyhow!("invalid 32-byte hex {hex_str}: {e}"))?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| anyhow!("expected 32 bytes, got {} from {hex_str}", v.len()))
}
