//! Read-only Soroban RPC client (no sequence/submit paths).

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use stellar_rpc_client::{Client as InnerClient, LedgerStart};
use stellar_xdr::curr::{LedgerEntryData, LedgerKey};

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

    /// Latest close time (Unix s) — ages oracle prices. `getLatestLedger` has no
    /// close time; pages one ledger via `getLedgers`.
    pub async fn latest_close_time(&self) -> Result<i64> {
        let sequence = self.latest_ledger().await?;
        let resp = self
            .inner
            .get_ledgers(LedgerStart::Ledger(sequence), Some(1), None)
            .await
            .context("get_ledgers")?;
        Ok(resp.latest_ledger_close_time)
    }

    /// Look up keys in request order; dedupes (RPC rejects duplicates).
    pub async fn get_ledger_entries(&self, keys: &[LedgerKey]) -> Result<Vec<LedgerEntryQuery>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
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

        let mut found = HashMap::with_capacity(resp.entries.len());
        for entry in &resp.entries {
            found.insert(&entry.key, entry);
        }

        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            let value = found.get(k).map(|entry| entry.val.clone());
            out.push(LedgerEntryQuery {
                key: k.clone(),
                value,
            });
        }
        Ok(out)
    }
}


#[derive(Debug, Clone)]
pub struct LedgerEntryQuery {
    pub key: LedgerKey,
    pub value: Option<LedgerEntryData>,
}
