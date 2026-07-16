//! Typed YAML configuration for the lending exporter.
//!
//! One file per network. Addresses mirror `configs/networks.json` (the deploy
//! artifact). The exporter only reads on-chain state, so there is no signer,
//! key-vault, or fee configuration here.

use std::net::SocketAddr;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::keys::contract_id_from_strkey;

/// Minimum scrape interval; tighter than this hammers the RPC for no dashboard
/// benefit at a 15s Prometheus scrape.
const MIN_SCRAPE_INTERVAL_SECONDS: u64 = 5;

#[derive(Debug, Clone, Deserialize)]
pub struct ExporterConfig {
    /// Informational network tag, surfaced as the `network` metric label.
    pub network: String,
    pub rpc: RpcConfig,
    pub contracts: ContractsConfig,
    #[serde(default)]
    pub markets: Vec<MarketConfig>,
    /// Spoke ids to scan for per-spoke flags/caps/usage.
    #[serde(default)]
    pub spokes: Vec<u32>,
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
    /// Multi-hub lending controller (`C...`).
    pub controller: String,
    /// Self-hosted price oracle adapter (`C...`); optional — only required for
    /// staleness reads against Xoxno/RedStone-sourced assets.
    #[serde(default)]
    pub xoxno_oracle_adapter: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketConfig {
    pub hub_id: u32,
    /// Asset SAC contract id (`C...`).
    pub asset: String,
    /// Human label used as the `symbol` metric label (e.g. `USDC`).
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
    /// Loads and validates a config file.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        let cfg: Self = serde_yaml::from_str(&raw)
            .with_context(|| format!("parse config {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        if self.rpc.url.trim().is_empty() {
            bail!("rpc.url is empty");
        }
        if self.rpc.passphrase.trim().is_empty() {
            bail!("rpc.passphrase is empty");
        }
        // Parsing the ids here surfaces a bad `C...` at boot rather than mid-scrape.
        contract_id_from_strkey(&self.contracts.controller)
            .context("contracts.controller is not a valid C... address")?;
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

    /// Parses contract addresses into raw 32-byte ids once, after validation.
    pub fn resolve(&self) -> Result<ResolvedContracts> {
        let controller = contract_id_from_strkey(&self.contracts.controller)?;
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
            oracle_adapter,
            markets,
        })
    }
}

/// Parsed contract ids ready for RPC calls.
#[derive(Debug, Clone)]
pub struct ResolvedContracts {
    pub controller: [u8; 32],
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
