use common::types::HubAssetKey;
use soroban_sdk::{vec, Address, Vec};

use crate::core::LendingTest;
use crate::helpers::{f64_to_i128, hub_asset};

impl LendingTest {
    pub fn withdraw(&mut self, user: &str, asset_name: &str, amount: f64) {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        self.withdraw_raw(user, asset_name, raw_amount);
    }

    pub fn withdraw_raw(&mut self, user: &str, asset_name: &str, amount: i128) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        let withdrawals: Vec<(HubAssetKey, i128)> =
            vec![&self.env, (hub_asset(asset_addr), amount)];
        ctrl.withdraw(&addr, &account_id, &withdrawals, &None);
    }

    /// Withdraws to a third-party recipient and returns the paid amounts.
    pub fn withdraw_to_raw(
        &mut self,
        user: &str,
        asset_name: &str,
        amount: i128,
        recipient: &Address,
    ) -> Vec<(HubAssetKey, i128)> {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        let withdrawals: Vec<(HubAssetKey, i128)> =
            vec![&self.env, (hub_asset(asset_addr), amount)];
        ctrl.withdraw(&addr, &account_id, &withdrawals, &Some(recipient.clone()))
    }

    /// Withdraws and returns the actual paid amounts per asset.
    pub fn withdraw_raw_returning(
        &mut self,
        user: &str,
        asset_name: &str,
        amount: i128,
    ) -> Vec<(HubAssetKey, i128)> {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        let withdrawals: Vec<(HubAssetKey, i128)> =
            vec![&self.env, (hub_asset(asset_addr), amount)];
        ctrl.withdraw(&addr, &account_id, &withdrawals, &None)
    }

    pub fn try_withdraw(
        &mut self,
        user: &str,
        asset_name: &str,
        amount: f64,
    ) -> Result<(), soroban_sdk::Error> {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let account_id = self.try_resolve_account_id(user)?;
        let addr = self.users.get(user).unwrap().address.clone();
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        let withdrawals: Vec<(HubAssetKey, i128)> =
            vec![&self.env, (hub_asset(asset_addr), raw_amount)];
        match ctrl.try_withdraw(&addr, &account_id, &withdrawals, &None) {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    /// Withdraw multiple assets in a single controller call.
    /// HF check runs once AFTER all withdrawals (if borrows exist).
    pub fn withdraw_bulk(&mut self, user: &str, assets: &[(&str, f64)]) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();

        let mut soroban_withdrawals: Vec<(HubAssetKey, i128)> = Vec::new(&self.env);
        for (asset_name, amount) in assets {
            let market = self.resolve_market(asset_name);
            let raw = f64_to_i128(*amount, market.decimals);
            soroban_withdrawals.push_back((hub_asset(market.asset.clone()), raw));
        }

        let ctrl = self.ctrl_client();
        ctrl.withdraw(&addr, &account_id, &soroban_withdrawals, &None);
    }

    /// Withdraw the entire position for an asset (passes amount=0 which means "withdraw all").
    pub fn withdraw_all(&mut self, user: &str, asset_name: &str) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        let withdrawals: Vec<(HubAssetKey, i128)> = vec![&self.env, (hub_asset(asset_addr), 0i128)];
        ctrl.withdraw(&addr, &account_id, &withdrawals, &None);
    }
}
