use controller::types::PositionMode;
use soroban_sdk::{vec, Address, Vec};

use crate::core::LendingTest;
use crate::helpers::f64_to_i128;

impl LendingTest {
    /// Supply tokens. Auto-creates user address and account on first call.
    /// Auto-mints tokens to the user before calling controller.
    pub fn supply(&mut self, user: &str, asset_name: &str, amount: f64) {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        self.supply_raw(user, asset_name, raw_amount);
    }

    /// Supply with raw i128 amount.
    pub fn supply_raw(&mut self, user: &str, asset_name: &str, amount: i128) {
        let addr = self.get_or_create_user(user);
        let market = self.resolve_market(asset_name);
        let asset_addr = market.asset.clone();

        market.token_admin.mint(&addr, &amount);

        let account_id = self.default_account_id_or_zero(user);

        let ctrl = self.ctrl_client();
        let assets: Vec<(Address, i128)> = vec![&self.env, (asset_addr, amount)];
        let returned_id = ctrl.supply(&addr, &account_id, &0u32, &assets);

        if account_id == 0 {
            self.register_account(user, returned_id, 0, PositionMode::Normal, false);
        }
    }

    /// Supply to a specific account.
    pub fn supply_to(&mut self, user: &str, account_id: u64, asset_name: &str, amount: f64) {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let addr = self.get_or_create_user(user);
        let market = self.resolve_market(asset_name);
        let asset_addr = market.asset.clone();
        market.token_admin.mint(&addr, &raw_amount);

        let ctrl = self.ctrl_client();
        let assets: Vec<(Address, i128)> = vec![&self.env, (asset_addr, raw_amount)];
        ctrl.supply(&addr, &account_id, &0u32, &assets);
    }

    /// Try supply -- returns Result instead of panicking.
    pub fn try_supply(
        &mut self,
        user: &str,
        asset_name: &str,
        amount: f64,
    ) -> Result<u64, soroban_sdk::Error> {
        self.try_supply_with_e_mode(user, asset_name, amount, 0)
    }

    /// Try supply with an explicit e-mode argument -- returns Result.
    /// Supply to an existing account owned by `target_user`, signed by `caller`.
    pub fn try_supply_to_account(
        &mut self,
        caller: &str,
        target_user: &str,
        asset_name: &str,
        amount: f64,
    ) -> Result<u64, soroban_sdk::Error> {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let caller_addr = self.get_or_create_user(caller);
        let market = self.resolve_market(asset_name);
        let asset_addr = market.asset.clone();
        market.token_admin.mint(&caller_addr, &raw_amount);

        let account_id = self.resolve_account_id(target_user);
        let ctrl = self.ctrl_client();
        let assets: Vec<(Address, i128)> = vec![&self.env, (asset_addr, raw_amount)];
        match ctrl.try_supply(&caller_addr, &account_id, &0u32, &assets) {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(err)) => Err(err),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    pub fn try_supply_with_e_mode(
        &mut self,
        user: &str,
        asset_name: &str,
        amount: f64,
        e_mode_category: u32,
    ) -> Result<u64, soroban_sdk::Error> {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let addr = self.get_or_create_user(user);
        let market = self.resolve_market(asset_name);
        let asset_addr = market.asset.clone();
        market.token_admin.mint(&addr, &raw_amount);

        let account_id = self.default_account_id_or_zero(user);

        let ctrl = self.ctrl_client();
        let assets: Vec<(Address, i128)> = vec![&self.env, (asset_addr, raw_amount)];
        match ctrl.try_supply(&addr, &account_id, &e_mode_category, &assets) {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(err)) => Err(err),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    /// Supply multiple assets in a single controller call.
    /// Auto-mints tokens for each asset. Auto-creates account if needed.
    pub fn supply_bulk(&mut self, user: &str, assets: &[(&str, f64)]) {
        let addr = self.get_or_create_user(user);

        let mut soroban_assets: Vec<(Address, i128)> = Vec::new(&self.env);
        for (asset_name, amount) in assets {
            let market = self.resolve_market(asset_name);
            let raw = f64_to_i128(*amount, market.decimals);
            market.token_admin.mint(&addr, &raw);
            soroban_assets.push_back((market.asset.clone(), raw));
        }

        let account_id = self.default_account_id_or_zero(user);

        let ctrl = self.ctrl_client();
        let returned_id = ctrl.supply(&addr, &account_id, &0u32, &soroban_assets);

        if account_id == 0 {
            self.register_account(user, returned_id, 0, PositionMode::Normal, false);
        }
    }
}
