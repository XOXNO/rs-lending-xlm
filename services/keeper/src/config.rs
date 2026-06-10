//! YAML configuration loader.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct KeeperConfig {
    pub network: String,
    pub rpc: RpcConfig,
    pub contracts: ContractsConfig,
    pub keyvault: KeyVaultConfig,
    pub signer: SignerConfig,
    pub fees: FeesConfig,
    pub schedule: ScheduleConfig,
    pub metrics: MetricsConfig,
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
    pub pool_wasm_hash: String,
    pub flash_loan_receiver: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyVaultConfig {
    pub url: String,
    pub secret_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignerConfig {
    #[serde(default = "default_derivation_path")]
    pub derivation_path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeesConfig {
    #[serde(default = "default_base_fee")]
    pub base_fee_stroops: u32,
    #[serde(default = "default_fee_multiplier")]
    pub resource_fee_multiplier: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleConfig {
    pub ttl_tick_seconds: u64,
    pub index_tick_seconds: u64,
    pub ttl_safety_margin_days: u32,
    pub asset_chunk: usize,
    pub max_txs_per_tick: usize,
    /// Enables the role-gated `update_indexes(assets)` sweep.
    #[serde(default)]
    pub enable_index_refresh: bool,
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

fn default_rpc_timeout() -> u64 {
    30
}
fn default_derivation_path() -> String {
    "m/44'/148'/0'".to_string()
}
fn default_base_fee() -> u32 {
    100
}
fn default_fee_multiplier() -> f64 {
    1.2
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_log_format() -> String {
    "json".to_string()
}

impl KeeperConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw =
            fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
        let cfg: KeeperConfig = serde_yaml::from_str(&raw)
            .with_context(|| format!("parse YAML at {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        if self.network.trim().is_empty() {
            return Err(anyhow!("config.network is empty"));
        }
        if self.rpc.url.trim().is_empty() {
            return Err(anyhow!("config.rpc.url is empty"));
        }
        if self.rpc.passphrase.trim().is_empty() {
            return Err(anyhow!("config.rpc.passphrase is empty"));
        }
        if !self.contracts.controller.starts_with('C') {
            return Err(anyhow!(
                "config.contracts.controller must be a C... address"
            ));
        }
        if !self.contracts.flash_loan_receiver.starts_with('C') {
            return Err(anyhow!(
                "config.contracts.flash_loan_receiver must be a C... address"
            ));
        }
        if self.contracts.pool_wasm_hash.len() != 64
            || hex::decode(&self.contracts.pool_wasm_hash).is_err()
        {
            return Err(anyhow!(
                "config.contracts.pool_wasm_hash must be a 32-byte hex string"
            ));
        }
        if self.schedule.asset_chunk == 0 || self.schedule.max_txs_per_tick == 0 {
            return Err(anyhow!(
                "config.schedule.asset_chunk and max_txs_per_tick must be > 0"
            ));
        }
        if self.fees.resource_fee_multiplier < 1.0 {
            return Err(anyhow!(
                "config.fees.resource_fee_multiplier must be >= 1.0"
            ));
        }
        Ok(())
    }

    pub fn safety_margin_ledgers(&self) -> u32 {
        self.schedule
            .ttl_safety_margin_days
            .saturating_mul(LEDGERS_PER_DAY)
    }
}

/// Approximate ledgers per day on Stellar.
pub const LEDGERS_PER_DAY: u32 = 17_280;
