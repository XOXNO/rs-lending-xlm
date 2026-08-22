use common::types::{HubAssetKey, SeizeMode};
use soroban_sdk::{vec, Vec};

use crate::context::LendingTest;
use crate::helpers::{hub_asset, HARNESS_HUB};
use crate::ops::internal::{amount_raw, asset_payment_vec, burn_prefund, map_try_ok_value};

impl LendingTest {
    pub fn liquidate(
        &mut self,
        liquidator: &str,
        target_user: &str,
        debt_asset: &str,
        amount: f64,
    ) {
        self.liquidate_with_mode(
            liquidator,
            target_user,
            debt_asset,
            amount,
            SeizeMode::Transfer,
        );
    }

    /// `liquidate` with an explicit seize mode. Returns the receiving account id, which is `0`
    /// for `SeizeMode::Transfer`.
    pub fn liquidate_with_mode(
        &mut self,
        liquidator: &str,
        target_user: &str,
        debt_asset: &str,
        amount: f64,
        seize_mode: SeizeMode,
    ) -> u64 {
        self.liquidate_core(
            liquidator,
            target_user,
            debt_asset,
            amount,
            HARNESS_HUB,
            seize_mode,
        )
    }

    pub fn liquidate_on_hub(
        &mut self,
        hub_id: u32,
        liquidator: &str,
        target_user: &str,
        debt_asset: &str,
        amount: f64,
    ) {
        self.liquidate_core(
            liquidator,
            target_user,
            debt_asset,
            amount,
            hub_id,
            SeizeMode::Transfer,
        );
    }

    /// Pre-funds the liquidator and repays `amount` of `debt_asset` on
    /// `hub_id`. Returns the receiving account id, `0` for `Transfer`.
    fn liquidate_core(
        &mut self,
        liquidator: &str,
        target_user: &str,
        debt_asset: &str,
        amount: f64,
        hub_id: u32,
        seize_mode: SeizeMode,
    ) -> u64 {
        let decimals = self.resolve_market(debt_asset).decimals;
        let raw_amount = amount_raw(amount, decimals);
        let asset_addr = self.resolve_asset(debt_asset);

        let liquidator_addr = self.get_or_create_user(liquidator);
        let account_id = self.resolve_account_id(target_user);

        self.resolve_market(debt_asset)
            .token_admin
            .mint(&liquidator_addr, &raw_amount);

        let ctrl = self.ctrl_client();
        let payments: Vec<(HubAssetKey, i128)> = vec![
            &self.env,
            (
                HubAssetKey {
                    hub_id,
                    asset: asset_addr,
                },
                raw_amount,
            ),
        ];
        ctrl.liquidate(&liquidator_addr, &account_id, &payments, &seize_mode)
    }

    pub fn try_liquidate(
        &mut self,
        liquidator: &str,
        target_user: &str,
        debt_asset: &str,
        amount: f64,
    ) -> Result<(), soroban_sdk::Error> {
        self.try_liquidate_with_mode(
            liquidator,
            target_user,
            debt_asset,
            amount,
            SeizeMode::Transfer,
        )
        .map(|_| ())
    }

    /// `try_liquidate` with an explicit seize mode. On success returns the receiving account
    /// id (`0` for `SeizeMode::Transfer`); on failure the liquidator's pre-funded balance is
    /// burned again so the harness's token books stay comparable across attempts.
    pub fn try_liquidate_with_mode(
        &mut self,
        liquidator: &str,
        target_user: &str,
        debt_asset: &str,
        amount: f64,
        seize_mode: SeizeMode,
    ) -> Result<u64, soroban_sdk::Error> {
        let decimals = self.resolve_market(debt_asset).decimals;
        let raw_amount = amount_raw(amount, decimals);
        let asset_addr = self.resolve_asset(debt_asset);

        let liquidator_addr = self.get_or_create_user(liquidator);
        let account_id = self.try_resolve_account_id(target_user)?;

        self.resolve_market(debt_asset)
            .token_admin
            .mint(&liquidator_addr, &raw_amount);

        let ctrl = self.ctrl_client();
        let payments = asset_payment_vec(&self.env, asset_addr.clone(), raw_amount);
        let res = map_try_ok_value(ctrl.try_liquidate(
            &liquidator_addr,
            &account_id,
            &payments,
            &seize_mode,
        ));
        if res.is_err() {
            burn_prefund(&self.env, &asset_addr, &liquidator_addr, raw_amount);
        }
        res
    }

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
        ctrl.liquidate(
            &liquidator_addr,
            &account_id,
            &payments,
            &SeizeMode::Transfer,
        );
    }
}
