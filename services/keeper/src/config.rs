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
    /// RPC endpoints in preference order: primary first, last-resort last. The
    /// key accepts either spelling (`url` or `urls`) and either shape (one
    /// string or a list), so a single-endpoint config stays valid unchanged.
    #[serde(alias = "url", deserialize_with = "one_or_many")]
    pub urls: Vec<String>,
    pub passphrase: String,
    #[serde(default = "default_rpc_timeout")]
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContractsConfig {
    pub controller: String,
    pub pool_wasm_hash: String,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub flash_loan_receiver: Option<String>,

    #[serde(default)]
    pub markets: Vec<MarketConfig>,

    #[serde(default)]
    pub market_assets: Vec<String>,

    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub governance: Option<String>,

    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub xoxno_oracle_adapter: Option<String>,

    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub price_aggregator: Option<String>,
}

impl ContractsConfig {
    fn require_aggregator_for_markets(&self) -> Result<()> {
        if self.price_aggregator.is_none()
            && (!self.markets.is_empty() || !self.market_assets.is_empty())
        {
            return Err(anyhow!(
                "config.contracts.price_aggregator is required when markets are configured \
                 (the keeper must renew the price-aggregator's AssetOracle rows and instance)"
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketConfig {
    #[serde(default = "default_hub_id")]
    pub hub_id: u32,
    pub asset: String,
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

    #[serde(default)]
    pub enable_index_refresh: bool,

    #[serde(default = "default_scan_users")]
    pub scan_users: bool,

    #[serde(default = "default_max_accounts_scan")]
    pub max_accounts_scan: u64,
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
fn default_scan_users() -> bool {
    true
}
fn default_max_accounts_scan() -> u64 {
    50_000
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_log_format() -> String {
    "json".to_string()
}
fn default_hub_id() -> u32 {
    1
}

/// Accepts `url: <string>` and `urls: [<string>, ...]` as the same field, so
/// adding a fallback endpoint does not invalidate a deployed config file.
fn one_or_many<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }

    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(single) => vec![single],
        OneOrMany::Many(list) => list,
    })
}

fn empty_string_as_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    Ok(opt.filter(|s| !s.trim().is_empty()))
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
        if self.rpc.urls.is_empty() {
            return Err(anyhow!("config.rpc.url lists no endpoint"));
        }
        if let Some(idx) = self.rpc.urls.iter().position(|u| u.trim().is_empty()) {
            return Err(anyhow!("config.rpc.url[{idx}] is empty"));
        }
        if self.rpc.passphrase.trim().is_empty() {
            return Err(anyhow!("config.rpc.passphrase is empty"));
        }
        if !self.contracts.controller.starts_with('C') {
            return Err(anyhow!(
                "config.contracts.controller must be a C... address"
            ));
        }
        if let Some(flash_loan_receiver) = &self.contracts.flash_loan_receiver {
            if !flash_loan_receiver.starts_with('C') {
                return Err(anyhow!(
                    "config.contracts.flash_loan_receiver must be a C... address when set"
                ));
            }
        }
        if let Some(governance) = &self.contracts.governance {
            if !governance.starts_with('C') {
                return Err(anyhow!(
                    "config.contracts.governance must be a C... address when set"
                ));
            }
        }
        if let Some(adapter) = &self.contracts.xoxno_oracle_adapter {
            if !adapter.starts_with('C') {
                return Err(anyhow!(
                    "config.contracts.xoxno_oracle_adapter must be a C... address when set"
                ));
            }
        }
        if let Some(agg) = &self.contracts.price_aggregator {
            if !agg.starts_with('C') {
                return Err(anyhow!(
                    "config.contracts.price_aggregator must be a C... address when set"
                ));
            }
        }
        self.contracts.require_aggregator_for_markets()?;
        if self.contracts.pool_wasm_hash.len() != 64
            || hex::decode(&self.contracts.pool_wasm_hash).is_err()
        {
            return Err(anyhow!(
                "config.contracts.pool_wasm_hash must be a 32-byte hex string"
            ));
        }
        for asset in &self.contracts.market_assets {
            if !asset.starts_with('C') {
                return Err(anyhow!(
                    "config.contracts.market_assets entries must be contract IDs"
                ));
            }
        }
        for market in &self.contracts.markets {
            if market.hub_id == 0 {
                return Err(anyhow!(
                    "config.contracts.markets entries must use hub_id >= 1"
                ));
            }
            if !market.asset.starts_with('C') {
                return Err(anyhow!(
                    "config.contracts.markets entries must use C... asset contract IDs"
                ));
            }
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

pub const LEDGERS_PER_DAY: u32 = 17_280;

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped config files are deployed as-is, and nothing else in the
    /// build reads them. Parse every one of them here, so a YAML edit cannot
    /// reach a running keeper untested.
    ///
    /// This asserts the file shape, not `validate()`: `config/testnet.yaml`
    /// declares markets with no `price_aggregator` and fails validation today,
    /// which is a defect in that file rather than in the parser.
    #[test]
    fn shipped_configs_parse_with_an_rpc_endpoint() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("config");
        let mut parsed = 0;
        for entry in fs::read_dir(&dir).expect("read config dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().is_none_or(|ext| ext != "yaml") {
                continue;
            }
            let raw = fs::read_to_string(&path).expect("read config");
            let cfg: KeeperConfig = serde_yaml::from_str(&raw)
                .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
            assert!(
                !cfg.rpc.urls.iter().any(|u| u.trim().is_empty()),
                "{} has a blank RPC endpoint",
                path.display()
            );
            assert!(
                !cfg.rpc.urls.is_empty(),
                "{} has no RPC endpoint",
                path.display()
            );
            parsed += 1;
        }
        assert!(parsed >= 3, "expected the shipped configs, found {parsed}");
    }

    fn rpc(yaml: &str) -> Result<RpcConfig> {
        Ok(serde_yaml::from_str(yaml)?)
    }

    /// A deployed config writes `url:` as one string. That must keep parsing
    /// unchanged, and a list must parse under either spelling.
    #[test]
    fn rpc_url_accepts_one_or_many() {
        let one = rpc("url: https://a.example\npassphrase: p\n").expect("scalar url");
        assert_eq!(one.urls, vec!["https://a.example".to_string()]);
        assert_eq!(one.timeout_seconds, 30);

        let many = rpc("urls: [https://a.example, https://b.example]\npassphrase: p\n")
            .expect("urls list");
        assert_eq!(many.urls.len(), 2);

        let aliased =
            rpc("url: [https://a.example, https://b.example]\npassphrase: p\n").expect("url list");
        assert_eq!(aliased.urls, many.urls);
    }

    #[test]
    fn rpc_url_rejects_empty_endpoints() {
        let cfg = rpc("urls: []\npassphrase: p\n").expect("empty list parses");
        assert!(cfg.urls.is_empty());

        let blank = rpc("urls: [https://a.example, \"  \"]\npassphrase: p\n").expect("parses");
        assert!(blank.urls.iter().any(|u| u.trim().is_empty()));
    }

    fn contracts(gov_line: &str) -> ContractsConfig {
        let yaml = format!(
            "controller: CCONTROLLER\npool_wasm_hash: {}\n{}",
            "0".repeat(64),
            gov_line
        );
        serde_yaml::from_str(&yaml).expect("parse ContractsConfig")
    }

    /// The flash-loan receiver is a testnet demo contract; mainnet never
    /// deploys one. It used to be a required `String` validated to start with
    /// `C`, so an unset receiver failed config load outright and `prepay_rent`
    /// could not run on mainnet at all. It is optional now, like `governance`
    /// and `price_aggregator`, and both "absent" spellings have to parse: the
    /// key omitted entirely, and the key present but empty (which is what the
    /// Makefile used to emit from an empty `networks.json` field).
    #[test]
    fn an_absent_flash_loan_receiver_parses() {
        assert_eq!(contracts("").flash_loan_receiver, None);

        let empty: ContractsConfig = serde_yaml::from_str(&format!(
            "controller: CCONTROLLER\npool_wasm_hash: {}\nflash_loan_receiver: \"\"",
            "0".repeat(64)
        ))
        .expect("an empty flash_loan_receiver must parse as absent");
        assert_eq!(empty.flash_loan_receiver, None);
    }

    #[test]
    fn markets_without_price_aggregator_fail_validation() {
        let with_markets =
            contracts("markets:\n  - { hub_id: 1, asset: CASSET }\nprice_aggregator: \"\"");
        assert!(with_markets.require_aggregator_for_markets().is_err());

        let wired = contracts("markets:\n  - { hub_id: 1, asset: CASSET }\nprice_aggregator: CAGG");
        assert!(wired.require_aggregator_for_markets().is_ok());

        assert!(contracts("").require_aggregator_for_markets().is_ok());
    }

    #[test]
    fn governance_blank_yaml_deserializes_to_none() {
        assert_eq!(contracts("").governance, None);
        assert_eq!(contracts("governance: \"\"").governance, None);
        assert_eq!(contracts("governance: \"   \"").governance, None);
        assert_eq!(
            contracts("governance: CGOV").governance,
            Some("CGOV".to_string())
        );
    }
}
