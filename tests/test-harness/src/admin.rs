use controller::types::{InterestRateModel, SpokeAssetArgs};
use soroban_sdk::Address;

use crate::context::LendingTest;
use crate::helpers::{hub_asset, HARNESS_HUB, HARNESS_SPOKE};
use crate::view::AssetConfigView;

impl LendingTest {
    pub fn admin(&self) -> Address {
        self.admin.clone()
    }

    pub fn controller_address(&self) -> Address {
        self.controller.clone()
    }

    pub fn pause(&self) {
        self.ctrl_client().pause();
    }

    pub fn unpause(&self) {
        self.ctrl_client().unpause();
    }

    pub fn set_accumulator(&self, addr: &Address) {
        self.ctrl_client().set_accumulator(addr);
    }

    /// Mutate current config: risk/`liquidation_fees` → base spoke via
    /// `edit_asset_in_spoke`; flash-loan fields → pool `MarketParamsRaw`.
    pub fn edit_asset_config(&self, asset_name: &str, f: impl FnOnce(&mut AssetConfigView)) {
        let asset = self.resolve_asset(asset_name);
        let mut config = self.get_asset_config(asset_name);
        f(&mut config);

        self.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
            hub_id: HARNESS_HUB,
            asset: asset.clone(),
            spoke_id: HARNESS_SPOKE,
            can_collateral: config.is_collateralizable,
            can_borrow: config.is_borrowable,
            paused: false,
            frozen: false,
            ltv: config.loan_to_value,
            threshold: config.liquidation_threshold,
            bonus: config.liquidation_bonus,
            liquidation_fees: config.liquidation_fees,
            supply_cap: 0,
            borrow_cap: 0,
            oracle_override: controller::types::MarketOracleConfigOption::None,
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
            params.flashloan_fee = config.flashloan_fee;
            self.env.storage().persistent().set(&key, &params);
        });
    }

    /// Set per-spoke `paused` (preserves risk). Blocks user verbs; liquidation stays reachable.
    pub fn set_spoke_asset_paused(&self, asset_name: &str, paused: bool) {
        let asset = self.resolve_asset(asset_name);
        let config = self.get_asset_config(asset_name);
        self.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
            hub_id: HARNESS_HUB,
            asset,
            spoke_id: HARNESS_SPOKE,
            can_collateral: config.is_collateralizable,
            can_borrow: config.is_borrowable,
            paused,
            frozen: false,
            ltv: config.loan_to_value,
            threshold: config.liquidation_threshold,
            bonus: config.liquidation_bonus,
            liquidation_fees: config.liquidation_fees,
            supply_cap: 0,
            borrow_cap: 0,
            oracle_override: controller::types::MarketOracleConfigOption::None,
        });
    }

    pub fn set_position_limits(&self, max_supply: u32, max_borrow: u32) {
        let limits = controller::types::PositionLimits {
            max_supply_positions: max_supply,
            max_borrow_positions: max_borrow,
        };
        self.ctrl_client().set_position_limits(&limits);
    }

    pub fn upgrade_pool_params(&self, asset_name: &str, params: InterestRateModel) {
        let asset = self.resolve_asset(asset_name);
        self.ctrl_client()
            .upgrade_liquidity_pool_params(&hub_asset(asset), &params);
    }

    pub fn remove_spoke_category(&self, category_id: u32) {
        self.ctrl_client().remove_spoke(&category_id);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_asset_to_spoke(
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
            hub_id: HARNESS_HUB,
            asset,
            spoke_id: category_id,
            can_collateral,
            can_borrow,
            paused: false,
            frozen: false,
            ltv,
            threshold,
            bonus,
            liquidation_fees: 0,
            supply_cap: 0,
            borrow_cap: 0,
            oracle_override: controller::types::MarketOracleConfigOption::None,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn edit_asset_in_spoke(
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
            hub_id: HARNESS_HUB,
            asset,
            spoke_id: category_id,
            can_collateral,
            can_borrow,
            paused: false,
            frozen: false,
            ltv,
            threshold,
            bonus,
            liquidation_fees: 0,
            supply_cap: 0,
            borrow_cap: 0,
            oracle_override: controller::types::MarketOracleConfigOption::None,
        });
    }

    pub fn remove_asset_from_spoke(&self, asset_name: &str, category_id: u32) {
        let asset = self.resolve_asset(asset_name);
        self.ctrl_client()
            .remove_asset_from_spoke(&hub_asset(asset), &category_id);
    }

    /// Like `edit_asset_in_spoke` with real supply/borrow caps (other helpers pass `0` = disabled).
    #[allow(clippy::too_many_arguments)]
    pub fn edit_asset_in_spoke_caps(
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
            hub_id: HARNESS_HUB,
            asset,
            spoke_id: category_id,
            can_collateral,
            can_borrow,
            paused: false,
            frozen: false,
            ltv,
            threshold,
            bonus,
            liquidation_fees: 0,
            supply_cap,
            borrow_cap,
            oracle_override: controller::types::MarketOracleConfigOption::None,
        });
    }
}
