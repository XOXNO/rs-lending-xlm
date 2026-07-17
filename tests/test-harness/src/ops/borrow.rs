use common::types::HubAssetKey;
use soroban_sdk::{vec, Vec};

use crate::core::LendingTest;
use crate::helpers::{f64_to_i128, hub_asset};

impl LendingTest {
    pub fn borrow(&mut self, user: &str, asset_name: &str, amount: f64) {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        self.borrow_raw(user, asset_name, raw_amount);
    }

    pub fn borrow_raw(&mut self, user: &str, asset_name: &str, amount: i128) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        let borrows: Vec<(HubAssetKey, i128)> = vec![&self.env, (hub_asset(asset_addr), amount)];
        ctrl.borrow(&addr, &account_id, &borrows, &None);
    }

    pub fn borrow_to(&mut self, user: &str, account_id: u64, asset_name: &str, amount: f64) {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let addr = self.users.get(user).unwrap().address.clone();
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        let borrows: Vec<(HubAssetKey, i128)> =
            vec![&self.env, (hub_asset(asset_addr), raw_amount)];
        ctrl.borrow(&addr, &account_id, &borrows, &None);
    }

    pub fn borrow_as_to(
        &mut self,
        caller: &str,
        account_id: u64,
        asset_name: &str,
        amount: f64,
        to: &str,
    ) {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let caller_addr = self.users.get(caller).unwrap().address.clone();
        let to_addr = self.users.get(to).unwrap().address.clone();
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        let borrows: Vec<(HubAssetKey, i128)> =
            vec![&self.env, (hub_asset(asset_addr), raw_amount)];
        ctrl.borrow(&caller_addr, &account_id, &borrows, &Some(to_addr));
    }

    pub fn try_borrow(
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
        let borrows: Vec<(HubAssetKey, i128)> =
            vec![&self.env, (hub_asset(asset_addr), raw_amount)];
        match ctrl.try_borrow(&addr, &account_id, &borrows, &None) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    /// Borrow multiple assets in a single controller call.
    /// HF check runs once AFTER all borrows (cumulative).
    pub fn borrow_bulk(&mut self, user: &str, assets: &[(&str, f64)]) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();

        let mut soroban_borrows: Vec<(HubAssetKey, i128)> = Vec::new(&self.env);
        for (asset_name, amount) in assets {
            let market = self.resolve_market(asset_name);
            let raw = f64_to_i128(*amount, market.decimals);
            soroban_borrows.push_back((hub_asset(market.asset.clone()), raw));
        }

        let ctrl = self.ctrl_client();
        ctrl.borrow(&addr, &account_id, &soroban_borrows, &None);
    }

    pub fn try_borrow_bulk(
        &mut self,
        user: &str,
        assets: &[(&str, f64)],
    ) -> Result<(), soroban_sdk::Error> {
        let account_id = self.try_resolve_account_id(user)?;
        let addr = self.users.get(user).unwrap().address.clone();

        let mut soroban_borrows: Vec<(HubAssetKey, i128)> = Vec::new(&self.env);
        for (asset_name, amount) in assets {
            let market = self.resolve_market(asset_name);
            let raw = f64_to_i128(*amount, market.decimals);
            soroban_borrows.push_back((hub_asset(market.asset.clone()), raw));
        }

        let ctrl = self.ctrl_client();
        match ctrl.try_borrow(&addr, &account_id, &soroban_borrows, &None) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }
}
