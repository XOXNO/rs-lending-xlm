//! Runtime oracle controls: mock reflector prices and on-chain market oracle strategy.

use crate::context::LendingTest;
use crate::helpers::HARNESS_HUB;
use crate::presets::TolerancePreset;
use controller::types::{
    MarketOracleConfig, MarketOracleConfigOption, OracleAssetRef, OraclePriceFluctuation,
    OracleReadMode, OracleSourceConfig, OracleSourceConfigOption, OracleStrategy, ReflectorBase,
    ReflectorSourceConfig, SpokeAssetArgs,
};
use soroban_sdk::{testutils::Address as _, Address};

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
                tolerance: preset.tolerance_bps,
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
                hub_asset: crate::helpers::hub_asset(asset.clone()),
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

    /// Point `asset_name` at a per-spoke `oracle_override` priced at
    /// `override_price_wad`, routed through the real `edit_asset_in_spoke` entry
    /// (arg -> validation -> storage). The override reads a fresh mock-reflector
    /// asset so the base token-rooted price stays untouched, proving the spoke
    /// reprices the asset independently. Returns the override price source asset.
    pub fn set_spoke_oracle_override(
        &self,
        asset_name: &str,
        spoke_id: u32,
        override_price_wad: i128,
    ) -> Address {
        let asset = self.resolve_asset(asset_name);
        let config = self.get_asset_config(asset_name);

        // Distinct price source so the override diverges from the token-rooted base.
        let override_source = Address::generate(&self.env);
        let reflector = self.mock_reflector_client();
        reflector.set_price(&override_source, &override_price_wad);
        reflector.set_twap_price(&override_source, &override_price_wad);

        let override_cfg = MarketOracleConfig {
            asset_decimals: config.asset_decimals,
            max_price_stale_seconds: 900,
            tolerance: OraclePriceFluctuation {
                upper_ratio_bps: 500,
                lower_ratio_bps: 500,
            },
            strategy: OracleStrategy::Single,
            primary: OracleSourceConfig::Reflector(ReflectorSourceConfig {
                contract: self.mock_reflector.clone(),
                asset: OracleAssetRef::Stellar(override_source.clone()),
                read_mode: OracleReadMode::Spot,
                decimals: 14,
                resolution_seconds: 300,
                base: ReflectorBase::Usd,
            }),
            anchor: OracleSourceConfigOption::None,
            min_sanity_price_wad: 1,
            max_sanity_price_wad: controller::constants::MAX_REASONABLE_PRICE_WAD,
        };

        self.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
            hub_id: HARNESS_HUB,
            asset,
            spoke_id,
            can_collateral: config.is_collateralizable,
            can_borrow: config.is_borrowable,
            ltv: config.loan_to_value,
            threshold: config.liquidation_threshold,
            bonus: config.liquidation_bonus,
            liquidation_fees: config.liquidation_fees,
            supply_cap: 0,
            borrow_cap: 0,
            oracle_override: MarketOracleConfigOption::Some(override_cfg),
        });

        override_source
    }

    pub fn set_oracle_single_spot(&self, asset_name: &str) {
        let asset = self.resolve_asset(asset_name);
        self.env.as_contract(&self.controller, || {
            let key = controller::types::ControllerKey::AssetOracle(asset.clone());
            let mut oracle: controller::types::MarketOracleConfig =
                self.env.storage().persistent().get(&key).unwrap();
            oracle.strategy = OracleStrategy::Single;
            oracle.primary = source_with_read_mode(&oracle.primary, OracleReadMode::Spot);
            oracle.anchor = OracleSourceConfigOption::None;
            self.env.storage().persistent().set(&key, &oracle);
        });
    }

    pub fn set_oracle_primary_anchor(&self, asset_name: &str) {
        let asset = self.resolve_asset(asset_name);
        self.env.as_contract(&self.controller, || {
            let key = controller::types::ControllerKey::AssetOracle(asset.clone());
            let mut oracle: controller::types::MarketOracleConfig =
                self.env.storage().persistent().get(&key).unwrap();
            oracle.strategy = OracleStrategy::PrimaryWithAnchor;
            oracle.primary = source_with_read_mode(&oracle.primary, OracleReadMode::Twap(3));
            oracle.anchor = OracleSourceConfigOption::Some(source_with_read_mode(
                &oracle.primary,
                OracleReadMode::Spot,
            ));
            self.env.storage().persistent().set(&key, &oracle);
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
            let key = controller::types::ControllerKey::AssetOracle(asset.clone());
            let mut oracle: controller::types::MarketOracleConfig =
                self.env.storage().persistent().get(&key).unwrap();
            oracle.strategy = OracleStrategy::PrimaryWithAnchor;
            oracle.primary = match oracle.primary {
                OracleSourceConfig::Reflector(mut source) => {
                    source.read_mode = OracleReadMode::Twap(3);
                    OracleSourceConfig::Reflector(source)
                }
                source => source,
            };
            oracle.anchor = OracleSourceConfigOption::Some(OracleSourceConfig::Reflector(
                ReflectorSourceConfig {
                    contract: dex_oracle,
                    asset: OracleAssetRef::Stellar(asset.clone()),
                    read_mode: OracleReadMode::Spot,
                    decimals: 14,
                    resolution_seconds: 300,
                    base: ReflectorBase::Usd,
                },
            ));
            self.env.storage().persistent().set(&key, &oracle);
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
