use crate::context::LendingTest;
use crate::presets::TolerancePreset;
use common::types::{OracleReadMode, OracleSourceConfig, OracleSourceConfigOption, OracleStrategy};

impl LendingTest {
    /// Set the oracle price for an asset. Use with usd(), usd_cents(), usd_frac().
    pub fn set_price(&mut self, asset_name: &str, price_wad: i128) {
        let market = self
            .markets
            .get_mut(asset_name)
            .unwrap_or_else(|| panic!("market '{}' not found", asset_name));
        let asset = market.asset.clone();
        market.price_wad = price_wad;

        let mock_reflector = self.mock_reflector_client();
        mock_reflector.set_price(&asset, &price_wad);
        mock_reflector.set_twap_price(&asset, &price_wad);
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

    /// Set oracle tolerance for an asset.
    /// Passes raw deviation BPS; controller computes ratio bounds.
    pub fn set_oracle_tolerance(&self, asset_name: &str, preset: TolerancePreset) {
        let asset = self.resolve_asset(asset_name);
        let ctrl = self.ctrl_client();
        ctrl.edit_oracle_tolerance(
            &self.admin,
            &asset,
            &preset.first_upper_bps,
            &preset.last_upper_bps,
        );
    }

    pub fn set_safe_price(
        &self,
        asset_name: &str,
        price_wad: i128,
        _within_first: bool,
        _within_second: bool,
    ) {
        let market = self.resolve_market(asset_name);
        let asset = market.asset.clone();

        let mock_reflector = self.mock_reflector_client();
        mock_reflector.set_twap_price(&asset, &price_wad);
    }

    pub fn set_oracle_single_spot(&self, asset_name: &str) {
        let asset = self.resolve_asset(asset_name);
        self.env.as_contract(&self.controller, || {
            let key = common::types::ControllerKey::Market(asset.clone());
            let mut market: common::types::MarketConfig =
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
            let key = common::types::ControllerKey::Market(asset.clone());
            let mut market: common::types::MarketConfig =
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
