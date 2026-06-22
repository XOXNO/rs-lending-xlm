use controller::types::InterestRateModel;
use soroban_sdk::Address;

use crate::context::LendingTest;

impl LendingTest {
    // Public accessors

    /// Expose admin address for direct controller calls.
    pub fn admin(&self) -> Address {
        self.admin.clone()
    }

    /// Expose controller address.
    pub fn controller_address(&self) -> Address {
        self.controller.clone()
    }
    // Pause / Unpause

    pub fn pause(&self) {
        self.ctrl_client().pause();
    }

    pub fn unpause(&self) {
        self.ctrl_client().unpause();
    }
    // Accumulator / Aggregator

    pub fn set_accumulator(&self, addr: &Address) {
        self.ctrl_client().set_accumulator(addr);
    }
    // Asset config editing

    /// Edit asset config at runtime via a closure that mutates the current config.
    pub fn edit_asset_config(
        &self,
        asset_name: &str,
        f: impl FnOnce(&mut controller::types::AssetConfigRaw),
    ) {
        let asset = self.resolve_asset(asset_name);
        let ctrl = self.ctrl_client();
        let mut config = ctrl.get_market_config(&asset).asset_config;
        f(&mut config);
        ctrl.edit_asset_config(&asset, &config);
    }
    // Position limits

    pub fn set_position_limits(&self, max_supply: u32, max_borrow: u32) {
        let limits = controller::types::PositionLimits {
            max_supply_positions: max_supply,
            max_borrow_positions: max_borrow,
        };
        self.ctrl_client().set_position_limits(&limits);
    }
    // Pool params upgrade

    pub fn upgrade_pool_params(&self, asset_name: &str, params: InterestRateModel) {
        let asset = self.resolve_asset(asset_name);
        self.ctrl_client()
            .upgrade_liquidity_pool_params(&asset, &params);
    }
    // E-mode management

    pub fn remove_e_mode_category(&self, category_id: u32) {
        self.ctrl_client().remove_e_mode_category(&category_id);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_asset_to_e_mode(
        &self,
        asset_name: &str,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
        ltv: u32,
        threshold: u32,
        bonus: u32,
    ) {
        let asset = self.resolve_asset(asset_name);
        self.ctrl_client().add_asset_to_e_mode_category(
            &asset,
            &category_id,
            &can_collateral,
            &can_borrow,
            &ltv,
            &threshold,
            &bonus,
            &0i128,
            &0i128,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn edit_asset_in_e_mode(
        &self,
        asset_name: &str,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
        ltv: u32,
        threshold: u32,
        bonus: u32,
    ) {
        let asset = self.resolve_asset(asset_name);
        self.ctrl_client().edit_asset_in_e_mode_category(
            &asset,
            &category_id,
            &can_collateral,
            &can_borrow,
            &ltv,
            &threshold,
            &bonus,
            &0i128,
            &0i128,
        );
    }

    pub fn remove_asset_from_e_mode(&self, asset_name: &str, category_id: u32) {
        let asset = self.resolve_asset(asset_name);
        self.ctrl_client()
            .remove_asset_from_e_mode(&asset, &category_id);
    }

}
