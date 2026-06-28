use common::types::HubAssetKey;
use soroban_sdk::Vec;

use crate::context::LendingTest;
use crate::helpers::hub_asset;
use crate::ops::internal::{amount_raw, asset_payment_vec, map_try_ok_unit};

impl LendingTest {
    /// Liquidate: proportional seizure across all collateral.
    /// Auto-mints debt tokens to the liquidator.
    pub fn liquidate(
        &mut self,
        liquidator: &str,
        target_user: &str,
        debt_asset: &str,
        amount: f64,
    ) {
        let decimals = self.resolve_market(debt_asset).decimals;
        let raw_amount = amount_raw(amount, decimals);
        let asset_addr = self.resolve_asset(debt_asset);

        let liquidator_addr = self.get_or_create_user(liquidator);
        let account_id = self.resolve_account_id(target_user);

        // Auto-mint debt tokens to liquidator
        self.resolve_market(debt_asset)
            .token_admin
            .mint(&liquidator_addr, &raw_amount);

        let ctrl = self.ctrl_client();
        let payments = asset_payment_vec(&self.env, asset_addr, raw_amount);
        ctrl.liquidate(&liquidator_addr, &account_id, &payments);
    }

    /// Try liquidate -- returns Result.
    pub fn try_liquidate(
        &mut self,
        liquidator: &str,
        target_user: &str,
        debt_asset: &str,
        amount: f64,
    ) -> Result<(), soroban_sdk::Error> {
        let decimals = self.resolve_market(debt_asset).decimals;
        let raw_amount = amount_raw(amount, decimals);
        let asset_addr = self.resolve_asset(debt_asset);

        let liquidator_addr = self.get_or_create_user(liquidator);
        let account_id = self.try_resolve_account_id(target_user)?;

        self.resolve_market(debt_asset)
            .token_admin
            .mint(&liquidator_addr, &raw_amount);

        let ctrl = self.ctrl_client();
        let payments = asset_payment_vec(&self.env, asset_addr, raw_amount);
        map_try_ok_unit(ctrl.try_liquidate(&liquidator_addr, &account_id, &payments))
    }

    /// Liquidate with multiple debt payments (different tokens).
    /// Auto-mints each debt token to the liquidator.
    pub fn liquidate_multi(&mut self, liquidator: &str, target_user: &str, debts: &[(&str, f64)]) {
        let liquidator_addr = self.get_or_create_user(liquidator);
        let account_id = self.resolve_account_id(target_user);

        let mut payments: Vec<(HubAssetKey, i128)> = Vec::new(&self.env);
        for &(asset_name, amount) in debts {
            let market = self.resolve_market(asset_name);
            let raw = amount_raw(amount, market.decimals);
            market.token_admin.mint(&liquidator_addr, &raw);
            payments.push_back((hub_asset(market.asset.clone()), raw));
        }

        let ctrl = self.ctrl_client();
        ctrl.liquidate(&liquidator_addr, &account_id, &payments);
    }
}
