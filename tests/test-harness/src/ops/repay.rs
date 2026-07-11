use common::types::HubAssetKey;
use soroban_sdk::{vec, Vec};

use crate::core::LendingTest;
use crate::helpers::{f64_to_i128, hub_asset};

impl LendingTest {
    pub fn repay(&mut self, user: &str, asset_name: &str, amount: f64) {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        self.repay_raw(user, asset_name, raw_amount);
    }

    pub fn repay_raw(&mut self, user: &str, asset_name: &str, amount: i128) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();
        let market = self.resolve_market(asset_name);
        let asset_addr = market.asset.clone();

        market.token_admin.mint(&addr, &amount);

        let ctrl = self.ctrl_client();
        let payments: Vec<(HubAssetKey, i128)> = vec![&self.env, (hub_asset(asset_addr), amount)];
        ctrl.repay(&addr, &account_id, &payments);
    }

    /// Repay multiple assets in a single controller call.
    /// Auto-mints tokens for each repayment.
    pub fn repay_bulk(&mut self, user: &str, assets: &[(&str, f64)]) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();

        let mut soroban_payments: Vec<(HubAssetKey, i128)> = Vec::new(&self.env);
        for (asset_name, amount) in assets {
            let market = self.resolve_market(asset_name);
            let raw = f64_to_i128(*amount, market.decimals);
            market.token_admin.mint(&addr, &raw);
            soroban_payments.push_back((hub_asset(market.asset.clone()), raw));
        }

        let ctrl = self.ctrl_client();
        ctrl.repay(&addr, &account_id, &soroban_payments);
    }

    pub fn try_repay(
        &mut self,
        user: &str,
        asset_name: &str,
        amount: f64,
    ) -> Result<(), soroban_sdk::Error> {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let account_id = self.try_resolve_account_id(user)?;
        let addr = self.users.get(user).unwrap().address.clone();
        let market = self.resolve_market(asset_name);
        let asset_addr = market.asset.clone();
        market.token_admin.mint(&addr, &raw_amount);

        let ctrl = self.ctrl_client();
        let payments: Vec<(HubAssetKey, i128)> =
            vec![&self.env, (hub_asset(asset_addr.clone()), raw_amount)];
        let res = match ctrl.try_repay(&addr, &account_id, &payments) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        };
        if res.is_err() {
            crate::ops::internal::burn_prefund(&self.env, &asset_addr, &addr, raw_amount);
        }
        res
    }
}
