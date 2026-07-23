//! YAML config for the lending exporter (one file per network).
//!
//! Addresses mirror `configs/networks.json`. Read-only: no signer/fees.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::keys::contract_id_from_strkey;

/// Floor on scrape interval (tighter only burns RPC).
const MIN_SCRAPE_INTERVAL_SECONDS: u64 = 5;

#[derive(Debug, Clone, Deserialize)]
pub struct ExporterConfig {
    /// `network` metric label.
    pub network: String,
    pub rpc: RpcConfig,
    pub contracts: ContractsConfig,
    #[serde(default)]
    pub markets: Vec<MarketConfig>,
    /// Spoke ids to scrape for flags/caps/usage.
    #[serde(default)]
    pub spokes: Vec<u32>,
    /// Optional hub display names (`hub` label); missing → `Hub {id}`.
    #[serde(default)]
    pub hubs: BTreeMap<u32, String>,
    /// Optional spoke display names (`spoke` label); missing → `Spoke {id}`.
    #[serde(default)]
    pub spoke_names: BTreeMap<u32, String>,
    #[serde(default = "default_scrape_interval")]
    pub scrape_interval_seconds: u64,
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub log: LogConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RpcConfig {
    pub url: String,
    pub passphrase: String,
    #[serde(default = "default_rpc_timeout")]
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContractsConfig {
    /// Lending controller (`C...`).
    pub controller: String,
    /// Price-aggregator (`C...`); owns token-rooted `AssetOracle` configs.
    #[serde(default)]
    pub price_aggregator: Option<String>,
    /// Oracle adapter (`C...`); needed for Xoxno/RedStone staleness reads.
    #[serde(default)]
    pub xoxno_oracle_adapter: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketConfig {
    pub hub_id: u32,
    /// Asset SAC (`C...`).
    pub asset: String,
    /// `symbol` metric label (e.g. `USDC`).
    pub symbol: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricsConfig {
    pub bind: SocketAddr,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_format")]
    pub format: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

fn default_scrape_interval() -> u64 {
    30
}
fn default_rpc_timeout() -> u64 {
    30
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_log_format() -> String {
    "json".to_string()
}

impl ExporterConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        let cfg: Self = serde_yaml::from_str(&raw)
            .with_context(|| format!("parse config {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn hub_name(&self, hub_id: u32) -> String {
        self.hubs
            .get(&hub_id)
            .cloned()
            .unwrap_or_else(|| format!("Hub {hub_id}"))
    }

    pub fn spoke_name(&self, spoke_id: u32) -> String {
        self.spoke_names
            .get(&spoke_id)
            .cloned()
            .unwrap_or_else(|| format!("Spoke {spoke_id}"))
    }

    fn validate(&self) -> Result<()> {
        if self.rpc.url.trim().is_empty() {
            bail!("rpc.url is empty");
        }
        if self.rpc.passphrase.trim().is_empty() {
            bail!("rpc.passphrase is empty");
        }
        // Fail bad `C...` at boot, not mid-scrape.
        contract_id_from_strkey(&self.contracts.controller)
            .context("contracts.controller is not a valid C... address")?;
        if let Some(agg) = &self.contracts.price_aggregator {
            if !agg.trim().is_empty() {
                contract_id_from_strkey(agg)
                    .context("contracts.price_aggregator is not a valid C... address")?;
            }
        }
        if let Some(adapter) = &self.contracts.xoxno_oracle_adapter {
            if !adapter.trim().is_empty() {
                contract_id_from_strkey(adapter)
                    .context("contracts.xoxno_oracle_adapter is not a valid C... address")?;
            }
        }
        for market in &self.markets {
            contract_id_from_strkey(&market.asset)
                .with_context(|| format!("market asset {} is not a valid C... address", market.asset))?;
            if market.symbol.trim().is_empty() {
                bail!("market {} (hub {}) has an empty symbol", market.asset, market.hub_id);
            }
        }
        if self.scrape_interval_seconds < MIN_SCRAPE_INTERVAL_SECONDS {
            bail!(
                "scrape_interval_seconds {} below minimum {}",
                self.scrape_interval_seconds,
                MIN_SCRAPE_INTERVAL_SECONDS
            );
        }
        Ok(())
    }

    pub fn resolve(&self) -> Result<ResolvedContracts> {
        let controller = contract_id_from_strkey(&self.contracts.controller)?;
        let price_aggregator = match &self.contracts.price_aggregator {
            Some(a) if !a.trim().is_empty() => Some(contract_id_from_strkey(a)?),
            _ => None,
        };
        let oracle_adapter = match &self.contracts.xoxno_oracle_adapter {
            Some(a) if !a.trim().is_empty() => Some(contract_id_from_strkey(a)?),
            _ => None,
        };
        let markets = self
            .markets
            .iter()
            .map(|m| {
                Ok(ResolvedMarket {
                    hub_id: m.hub_id,
                    asset_id: contract_id_from_strkey(&m.asset)?,
                    asset_strkey: m.asset.clone(),
                    symbol: m.symbol.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ResolvedContracts {
            controller,
            price_aggregator,
            oracle_adapter,
            markets,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedContracts {
    pub controller: [u8; 32],
    pub price_aggregator: Option<[u8; 32]>,
    pub oracle_adapter: Option<[u8; 32]>,
    pub markets: Vec<ResolvedMarket>,
}

#[derive(Debug, Clone)]
pub struct ResolvedMarket {
    pub hub_id: u32,
    pub asset_id: [u8; 32],
    pub asset_strkey: String,
    pub symbol: String,
}
