use soroban_sdk::{vec, Address, Vec};

use crate::context::LendingTest;
use crate::helpers::f64_to_i128;

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
        let raw_amount = f64_to_i128(amount, decimals);

        let liquidator_addr = self.get_or_create_user(liquidator);
        let account_id = self.resolve_account_id(target_user);
        let market = self.resolve_market(debt_asset);
        let asset_addr = market.asset.clone();

        // Auto-mint debt tokens to liquidator
        market.token_admin.mint(&liquidator_addr, &raw_amount);

        let ctrl = self.ctrl_client();
        let payments: Vec<(Address, i128)> = vec![&self.env, (asset_addr, raw_amount)];
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
        let raw_amount = f64_to_i128(amount, decimals);

        let liquidator_addr = self.get_or_create_user(liquidator);
        let account_id = self.try_resolve_account_id(target_user)?;
        let market = self.resolve_market(debt_asset);
        let asset_addr = market.asset.clone();

        market.token_admin.mint(&liquidator_addr, &raw_amount);

        let ctrl = self.ctrl_client();
        let payments: Vec<(Address, i128)> = vec![&self.env, (asset_addr, raw_amount)];
        match ctrl.try_liquidate(&liquidator_addr, &account_id, &payments) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    /// Liquidate with multiple debt payments (different tokens).
    /// Auto-mints each debt token to the liquidator.
    pub fn liquidate_multi(&mut self, liquidator: &str, target_user: &str, debts: &[(&str, f64)]) {
        let liquidator_addr = self.get_or_create_user(liquidator);
        let account_id = self.resolve_account_id(target_user);

        let mut payments: Vec<(Address, i128)> = Vec::new(&self.env);
        for &(asset_name, amount) in debts {
            let market = self.resolve_market(asset_name);
            let raw = f64_to_i128(amount, market.decimals);
            market.token_admin.mint(&liquidator_addr, &raw);
            payments.push_back((market.asset.clone(), raw));
        }

        let ctrl = self.ctrl_client();
        ctrl.liquidate(&liquidator_addr, &account_id, &payments);
    }

    /// Keeper: clean bad debt for a user's account.
    pub fn clean_bad_debt(&self, target_user: &str) {
        let account_id = self.resolve_account_id(target_user);
        let ctrl = self.ctrl_client();
        ctrl.clean_bad_debt(&self.keeper, &account_id);
    }
}
