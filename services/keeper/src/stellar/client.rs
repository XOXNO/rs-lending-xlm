use anyhow::{anyhow, Context, Result};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use stellar_rpc_client::{AuthMode, Client as InnerClient, SimulateTransactionResponse};
use stellar_xdr::{
    AccountId, LedgerEntryData, LedgerKey, MuxedAccount, PublicKey, ScContractInstance,
    TransactionEnvelope, Uint256,
};
use tracing::warn;

use crate::config::RpcConfig;

struct Endpoint {
    url: String,
    client: InnerClient,
}

/// One RPC surface over an ordered endpoint list. A request that fails on the
/// active endpoint is retried on the next one, and the endpoint that answers
/// becomes active for later requests, including the submission that follows a
/// read and a simulation.
pub struct RpcClient {
    endpoints: Vec<Endpoint>,
    active: AtomicUsize,
}

impl RpcClient {
    pub fn new(cfg: &RpcConfig) -> Result<Self> {
        let endpoints = cfg
            .urls
            .iter()
            .map(|url| {
                let client =
                    InnerClient::new(url).with_context(|| format!("connect RPC at {url}"))?;
                Ok(Endpoint {
                    url: url.clone(),
                    client,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if endpoints.is_empty() {
            return Err(anyhow!("rpc.url lists no endpoint"));
        }
        Ok(Self {
            endpoints,
            active: AtomicUsize::new(0),
        })
    }

    /// The endpoint that answered last. Transaction submission uses this
    /// directly instead of `try_all`: a submission is not safe to replay on a
    /// second node from here, because a send that fails after the network
    /// accepted it would be retried against a node that has not yet seen it.
    /// Failing the job and letting the next tick rebuild it is the safe path.
    pub fn inner(&self) -> &InnerClient {
        &self.endpoints[self.active_index()].client
    }

    fn active_index(&self) -> usize {
        self.active.load(Ordering::Relaxed) % self.endpoints.len()
    }

    /// Runs `f` against the active endpoint and, on error, against every other
    /// endpoint in configured order. Only idempotent requests belong here: a
    /// failover repeats the request verbatim on another node.
    async fn try_all<'a, T, F, Fut>(&'a self, what: &str, f: F) -> Result<T>
    where
        F: Fn(&'a InnerClient) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let total = self.endpoints.len();
        let start = self.active_index();
        let mut last_err: Option<anyhow::Error> = None;

        for offset in 0..total {
            let idx = (start + offset) % total;
            let endpoint = &self.endpoints[idx];
            match f(&endpoint.client).await {
                Ok(value) => {
                    if idx != start {
                        self.active.store(idx, Ordering::Relaxed);
                        warn!(
                            target: "keeper.rpc",
                            request = what,
                            from = %self.endpoints[start].url,
                            to = %endpoint.url,
                            "RPC failover"
                        );
                    }
                    return Ok(value);
                }
                Err(e) => {
                    warn!(
                        target: "keeper.rpc",
                        request = what,
                        url = %endpoint.url,
                        error = %e,
                        "RPC endpoint failed"
                    );
                    last_err = Some(e);
                }
            }
        }

        Err(last_err
            .unwrap_or_else(|| anyhow!("no RPC endpoint was tried"))
            .context(format!("{what} failed on all {total} RPC endpoints")))
    }

    pub async fn simulate(
        &self,
        envelope: &TransactionEnvelope,
        auth_mode: Option<AuthMode>,
    ) -> Result<SimulateTransactionResponse> {
        self.try_all("simulate_transaction_envelope", |client| async {
            client
                .simulate_transaction_envelope(envelope, copy_auth_mode(&auth_mode))
                .await
                .context("simulate_transaction_envelope")
        })
        .await
    }

    pub async fn latest_ledger(&self) -> Result<u32> {
        let resp = self
            .try_all("get_latest_ledger", |client| async {
                client
                    .get_latest_ledger()
                    .await
                    .context("get_latest_ledger")
            })
            .await?;
        Ok(resp.sequence)
    }

    pub async fn get_contract_instance(
        &self,
        contract_id: &[u8; 32],
    ) -> Result<ScContractInstance> {
        self.try_all("get_contract_instance", |client| async {
            client
                .get_contract_instance(contract_id)
                .await
                .with_context(|| {
                    format!(
                        "get_contract_instance({})",
                        stellar_strkey::Contract(*contract_id)
                    )
                })
        })
        .await
    }

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
            .try_all("get_full_ledger_entries", |client| async {
                client
                    .get_full_ledger_entries(&unique)
                    .await
                    .context("get_full_ledger_entries")
            })
            .await?;

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
            .try_all("get_account", |client| async {
                client
                    .get_account(account_strkey)
                    .await
                    .with_context(|| format!("get_account({account_strkey})"))
            })
            .await?;
        Ok(entry.seq_num.0)
    }
}

/// `AuthMode` has no `Clone`, and a failover needs one value per attempt.
fn copy_auth_mode(mode: &Option<AuthMode>) -> Option<AuthMode> {
    mode.as_ref().map(|mode| match mode {
        AuthMode::Enforce => AuthMode::Enforce,
        AuthMode::Record => AuthMode::Record,
        AuthMode::RecordAllowNonRoot => AuthMode::RecordAllowNonRoot,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    fn client_with(urls: &[&str]) -> RpcClient {
        RpcClient::new(&RpcConfig {
            urls: urls.iter().map(|u| (*u).to_string()).collect(),
            passphrase: "Test SDF Network ; September 2015".to_string(),
            timeout_seconds: 30,
        })
        .expect("build client")
    }

    /// `try_all` walks the endpoints in configured order starting at the active
    /// one, and the endpoint that answers becomes active. `inner()` follows, so
    /// the submission after a failed-over read reaches the same node.
    #[tokio::test]
    async fn failover_walks_in_order_and_sticks() {
        let client = client_with(&[
            "http://127.0.0.1:1",
            "http://127.0.0.1:2",
            "http://127.0.0.1:3",
        ]);
        assert_eq!(client.active_index(), 0);

        let attempts = AtomicU32::new(0);
        let value = client
            .try_all("test", |_| async {
                // Fail on endpoint 0 and 1, answer on endpoint 2.
                match attempts.fetch_add(1, Ordering::Relaxed) {
                    0 | 1 => Err(anyhow!("endpoint down")),
                    n => Ok(n),
                }
            })
            .await
            .expect("third endpoint answers");

        assert_eq!(value, 2);
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
        assert_eq!(client.active_index(), 2);

        // The next request starts at the endpoint that just answered.
        let first_tried = client
            .try_all("test", |c| async {
                Ok(std::ptr::eq(c, &client.endpoints[2].client))
            })
            .await
            .expect("active endpoint answers");
        assert!(first_tried);
    }

    /// A request that no endpoint can serve reports the whole list, not just
    /// the last node, so an operator sees an outage rather than one bad host.
    #[tokio::test]
    async fn all_endpoints_down_is_an_error() {
        let client = client_with(&["http://127.0.0.1:1", "http://127.0.0.1:2"]);
        let err = client
            .try_all("test", |_| async { Err::<(), _>(anyhow!("endpoint down")) })
            .await
            .expect_err("no endpoint answers");
        assert!(err.to_string().contains("all 2 RPC endpoints"), "{err}");
        // A total outage leaves the active endpoint where it was.
        assert_eq!(client.active_index(), 0);
    }

    #[test]
    fn a_single_endpoint_still_works() {
        let client = client_with(&["http://127.0.0.1:1"]);
        assert_eq!(client.endpoints.len(), 1);
        assert_eq!(client.active_index(), 0);
    }
}
