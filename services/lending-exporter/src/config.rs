use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::keys::contract_id_from_strkey;

const MIN_SCRAPE_INTERVAL_SECONDS: u64 = 5;

#[derive(Debug, Clone, Deserialize)]
pub struct ExporterConfig {
    pub network: String,
    pub rpc: RpcConfig,
    pub contracts: ContractsConfig,
    #[serde(default)]
    pub markets: Vec<MarketConfig>,

    #[serde(default)]
    pub spokes: Vec<u32>,

    #[serde(default)]
    pub hubs: BTreeMap<u32, String>,

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
    pub controller: String,

    #[serde(default)]
    pub price_aggregator: Option<String>,

    #[serde(default)]
    pub xoxno_oracle_adapter: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketConfig {
    pub hub_id: u32,

    pub asset: String,

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
        let mut cfg: Self = serde_yaml::from_str(&raw)
            .with_context(|| format!("parse config {}", path.display()))?;
        cfg.apply_environment_overrides();
        cfg.validate()?;
        Ok(cfg)
    }

    /// Deployment-specific addresses stay outside the immutable image. This is
    /// primarily for mainnet, whose controller is intentionally not committed
    /// before deployment.
    fn apply_environment_overrides(&mut self) {
        override_nonempty("EXPORTER_RPC_URL", &mut self.rpc.url);
        override_nonempty("EXPORTER_CONTROLLER", &mut self.contracts.controller);
        override_optional(
            "EXPORTER_PRICE_AGGREGATOR",
            &mut self.contracts.price_aggregator,
        );
        override_optional(
            "EXPORTER_XOXNO_ORACLE_ADAPTER",
            &mut self.contracts.xoxno_oracle_adapter,
        );
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
            contract_id_from_strkey(&market.asset).with_context(|| {
                format!("market asset {} is not a valid C... address", market.asset)
            })?;
            if market.symbol.trim().is_empty() {
                bail!(
                    "market {} (hub {}) has an empty symbol",
                    market.asset,
                    market.hub_id
                );
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

fn override_nonempty(name: &str, target: &mut String) {
    if let Ok(value) = std::env::var(name) {
        if !value.trim().is_empty() {
            *target = value;
        }
    }
}

fn override_optional(name: &str, target: &mut Option<String>) {
    if let Ok(value) = std::env::var(name) {
        *target = if value.trim().is_empty() {
            None
        } else {
            Some(value)
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_environment_overrides_replace_only_the_configured_values() {
        let mut cfg = ExporterConfig {
            network: "mainnet".to_string(),
            rpc: RpcConfig {
                url: "https://default-rpc.example".to_string(),
                passphrase: "network".to_string(),
                timeout_seconds: 30,
            },
            contracts: ContractsConfig {
                controller: "".to_string(),
                price_aggregator: Some("CDEFAULT".to_string()),
                xoxno_oracle_adapter: None,
            },
            markets: Vec::new(),
            spokes: Vec::new(),
            hubs: BTreeMap::new(),
            spoke_names: BTreeMap::new(),
            scrape_interval_seconds: 30,
            metrics: MetricsConfig {
                bind: "127.0.0.1:9110".parse().unwrap(),
            },
            log: LogConfig::default(),
        };

        std::env::set_var("EXPORTER_RPC_URL", "https://override-rpc.example");
        std::env::set_var("EXPORTER_CONTROLLER", "COVERRIDE");
        std::env::set_var("EXPORTER_PRICE_AGGREGATOR", "");
        std::env::set_var("EXPORTER_XOXNO_ORACLE_ADAPTER", "CADAPTER");
        cfg.apply_environment_overrides();
        std::env::remove_var("EXPORTER_RPC_URL");
        std::env::remove_var("EXPORTER_CONTROLLER");
        std::env::remove_var("EXPORTER_PRICE_AGGREGATOR");
        std::env::remove_var("EXPORTER_XOXNO_ORACLE_ADAPTER");

        assert_eq!(cfg.rpc.url, "https://override-rpc.example");
        assert_eq!(cfg.contracts.controller, "COVERRIDE");
        assert_eq!(cfg.contracts.price_aggregator, None);
        assert_eq!(
            cfg.contracts.xoxno_oracle_adapter.as_deref(),
            Some("CADAPTER")
        );
    }
}
