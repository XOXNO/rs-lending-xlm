use controller::types::{InterestRateModel, SpokeAssetArgs};
use soroban_sdk::Address;

use crate::context::LendingTest;
use crate::helpers::hub_asset;
use crate::view::AssetConfigView;

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
    /// Risk parameters write back to the general spoke 0; flash-loan eligibility/fee
    /// write directly to the pool `MarketParamsRaw` (no endpoint toggles them).
    pub fn edit_asset_config(&self, asset_name: &str, f: impl FnOnce(&mut AssetConfigView)) {
        let asset = self.resolve_asset(asset_name);
        let mut config = self.get_asset_config(asset_name);
        f(&mut config);

        self.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
            asset: asset.clone(),
            spoke_id: 0,
            can_collateral: config.is_collateralizable,
            can_borrow: config.is_borrowable,
            ltv: config.loan_to_value_bps,
            threshold: config.liquidation_threshold_bps,
            bonus: config.liquidation_bonus_bps,
            supply_cap: 0,
            borrow_cap: 0,
        });

        let pool = self.get_pool_address(asset_name);
        self.env.as_contract(&pool, || {
            let key = controller::types::PoolKey::Params(hub_asset(asset.clone()));
            let mut params: controller::types::MarketParamsRaw = self
                .env
                .storage()
                .persistent()
                .get(&key)
                .expect("pool params must exist");
            params.is_flashloanable = config.is_flashloanable;
            params.flashloan_fee_bps = config.flashloan_fee_bps;
            self.env.storage().persistent().set(&key, &params);
        });
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
        self.ctrl_client().remove_spoke(&category_id);
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
        self.ctrl_client().add_asset_to_spoke(&SpokeAssetArgs {
            asset,
            spoke_id: category_id,
            can_collateral,
            can_borrow,
            ltv,
            threshold,
            bonus,
            supply_cap: 0,
            borrow_cap: 0,
        });
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
        self.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
            asset,
            spoke_id: category_id,
            can_collateral,
            can_borrow,
            ltv,
            threshold,
            bonus,
            supply_cap: 0,
            borrow_cap: 0,
        });
    }

    pub fn remove_asset_from_e_mode(&self, asset_name: &str, category_id: u32) {
        let asset = self.resolve_asset(asset_name);
        self.ctrl_client()
            .remove_asset_from_spoke(&asset, &category_id);
    }

    /// Edit a spoke asset with explicit spoke supply/borrow caps. Mirrors
    /// `edit_asset_in_e_mode` but forwards real cap values instead of the
    /// hardcoded `0` (disabled) the other helpers use, so cap-bound preview
    /// branches become reachable from tests.
    #[allow(clippy::too_many_arguments)]
    pub fn edit_asset_in_e_mode_caps(
        &self,
        asset_name: &str,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
        ltv: u32,
        threshold: u32,
        bonus: u32,
        supply_cap: i128,
        borrow_cap: i128,
    ) {
        let asset = self.resolve_asset(asset_name);
        self.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
            asset,
            spoke_id: category_id,
            can_collateral,
            can_borrow,
            ltv,
            threshold,
            bonus,
            supply_cap,
            borrow_cap,
        });
    }
}
