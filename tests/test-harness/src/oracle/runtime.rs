//! Runtime oracle controls: mock reflector prices and on-chain market oracle strategy.

use crate::context::LendingTest;
use crate::presets::TolerancePreset;
use controller::types::{
    OracleAssetRef, OracleReadMode, OracleSourceConfig, OracleSourceConfigOption, OracleStrategy,
    ReflectorBase, ReflectorSourceConfig,
};
use soroban_sdk::Address;

impl LendingTest {
    /// Set the oracle price for an asset. Use with usd(), usd_cents(), usd_frac().
    pub fn set_price(&mut self, asset_name: &str, price_wad: i128) {
        let market = self
            .markets
            .get_mut(asset_name)
            .unwrap_or_else(|| panic!("market '{}' not found", asset_name));
        let asset = market.asset.clone();
        market.price_wad = price_wad;
        self.push_oracle_prices(&asset, price_wad);
    }

    /// Refresh mock reflector spot + TWAP from each market's stored `price_wad`.
    pub fn refresh_oracle_prices(&self) {
        for market in self.markets.values() {
            self.push_oracle_prices(&market.asset, market.price_wad);
        }
    }

    pub(crate) fn push_oracle_prices(&self, asset: &Address, price_wad: i128) {
        let mock_reflector = self.mock_reflector_client();
        mock_reflector.set_price(asset, &price_wad);
        mock_reflector.set_twap_price(asset, &price_wad);
    }

    /// Set the raw WAD price for an asset (alias for set_price).
    pub fn set_price_raw(&mut self, asset_name: &str, price_wad: i128) {
        self.set_price(asset_name, price_wad);
    }

    /// Batch-update prices for multiple assets.
    pub fn set_prices(&mut self, pairs: &[(&str, i128)]) {
        for (asset_name, price_wad) in pairs {
            self.set_price(asset_name, *price_wad);
        }
    }

    pub fn set_oracle_tolerance(&self, asset_name: &str, preset: TolerancePreset) {
        let asset = self.resolve_asset(asset_name);
        use governance::op::{AdminOperation, EditToleranceArgs};
        self.gov_client().execute_immediate(
            &self.admin,
            &AdminOperation::EditOracleTolerance(EditToleranceArgs {
                asset,
                first_tolerance: preset.first_upper_bps,
                last_tolerance: preset.last_upper_bps,
            }),
        );
    }

    /// Configure a market oracle through the governance forwarder, which
    /// probes the mock oracles, validates the input, and forwards the
    /// resolved config to the controller's thin setter.
    pub fn configure_market_oracle(
        &self,
        asset: &Address,
        input: &controller::types::MarketOracleConfigInput,
    ) {
        use governance::op::{AdminOperation, ConfigureOracleArgs};
        self.gov_client().execute_immediate(
            &self.admin,
            &AdminOperation::ConfigureMarketOracle(ConfigureOracleArgs {
                asset: asset.clone(),
                cfg: input.clone(),
            }),
        );
    }

    /// Set the TWAP ("safe") leg for dual-source tolerance tests.
    pub fn set_safe_price(
        &self,
        asset_name: &str,
        price_wad: i128,
        _within_first: bool,
        _within_second: bool,
    ) {
        let asset = self.resolve_market(asset_name).asset.clone();
        self.mock_reflector_client()
            .set_twap_price(&asset, &price_wad);
    }

    pub fn set_oracle_single_spot(&self, asset_name: &str) {
        let asset = self.resolve_asset(asset_name);
        self.env.as_contract(&self.controller, || {
            let key = controller::types::ControllerKey::Market(asset.clone());
            let mut market: controller::types::MarketConfig =
                self.env.storage().persistent().get(&key).unwrap();
            market.oracle_config.strategy = OracleStrategy::Single;
            market.oracle_config.primary =
                source_with_read_mode(&market.oracle_config.primary, OracleReadMode::Spot);
            market.oracle_config.anchor = OracleSourceConfigOption::None;
            self.env.storage().persistent().set(&key, &market);
        });
    }

    pub fn set_oracle_primary_anchor(&self, asset_name: &str) {
        let asset = self.resolve_asset(asset_name);
        self.env.as_contract(&self.controller, || {
            let key = controller::types::ControllerKey::Market(asset.clone());
            let mut market: controller::types::MarketConfig =
                self.env.storage().persistent().get(&key).unwrap();
            market.oracle_config.strategy = OracleStrategy::PrimaryWithAnchor;
            market.oracle_config.primary =
                source_with_read_mode(&market.oracle_config.primary, OracleReadMode::Twap(3));
            market.oracle_config.anchor = OracleSourceConfigOption::Some(source_with_read_mode(
                &market.oracle_config.primary,
                OracleReadMode::Spot,
            ));
            self.env.storage().persistent().set(&key, &market);
        });
    }

    /// Alias for dual-source tolerance tests: primary TWAP + anchor spot.
    pub fn enable_dual_source_oracle(&self, asset_name: &str) {
        self.set_oracle_primary_anchor(asset_name);
    }

    /// Wire a separate DEX reflector as anchor spot for dual-source repricing tests.
    pub fn set_dual_oracle_dex_anchor(&self, asset_name: &str, dex_oracle: Address) {
        let asset = self.resolve_asset(asset_name);
        self.env.as_contract(&self.controller, || {
            let key = controller::types::ControllerKey::Market(asset.clone());
            let mut market: controller::types::MarketConfig =
                self.env.storage().persistent().get(&key).unwrap();
            market.oracle_config.strategy = OracleStrategy::PrimaryWithAnchor;
            market.oracle_config.primary = match market.oracle_config.primary {
                OracleSourceConfig::Reflector(mut source) => {
                    source.read_mode = OracleReadMode::Twap(3);
                    OracleSourceConfig::Reflector(source)
                }
                source => source,
            };
            market.oracle_config.anchor = OracleSourceConfigOption::Some(
                OracleSourceConfig::Reflector(ReflectorSourceConfig {
                    contract: dex_oracle,
                    asset: OracleAssetRef::Stellar(asset.clone()),
                    read_mode: OracleReadMode::Spot,
                    decimals: 14,
                    resolution_seconds: 300,
                    base: ReflectorBase::Usd,
                }),
            );
            self.env.storage().persistent().set(&key, &market);
        });
    }
}

fn source_with_read_mode(
    source: &OracleSourceConfig,
    read_mode: OracleReadMode,
) -> OracleSourceConfig {
    match source {
        OracleSourceConfig::Reflector(config) => {
            let mut config = config.clone();
            config.read_mode = read_mode;
            OracleSourceConfig::Reflector(config)
        }
        OracleSourceConfig::RedStone(config) => OracleSourceConfig::RedStone(config.clone()),
    }
}
