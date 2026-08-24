use controller::types::{PositionMode, StrategySwap};
use soroban_sdk::{Address, Bytes, Vec};

use crate::core::{AccountEntry, LendingTest};
use crate::helpers::{f64_to_i128, hub_asset, HARNESS_SPOKE};
use crate::strategy::swap::build_aggregator_swap;

impl LendingTest {
    pub fn fund_router(&self, asset_name: &str, amount: f64) {
        let market = self.resolve_market(asset_name);
        let raw = f64_to_i128(amount, market.decimals);
        market.token_admin.mint(&self.aggregator, &raw);
    }

    pub fn fund_router_raw(&self, asset_name: &str, amount: i128) {
        let market = self.resolve_market(asset_name);
        market.token_admin.mint(&self.aggregator, &amount);
    }

    pub fn mock_swap_steps(
        &self,
        token_in: &str,
        token_out: &str,
        _price_wad: i128,
    ) -> StrategySwap {
        build_aggregator_swap(self, token_in, token_out, 0, 1)
    }

    pub fn multiply(
        &mut self,
        user: &str,
        collateral_asset: &str,
        debt_amount: f64,
        debt_asset: &str,
        mode: PositionMode,
        steps: &StrategySwap,
    ) -> u64 {
        self.try_multiply(user, collateral_asset, debt_amount, debt_asset, mode, steps)
            .expect("multiply")
    }

    #[allow(clippy::too_many_arguments)]
    pub fn try_multiply_with_category(
        &mut self,
        user: &str,
        category: u32,
        collateral_asset: &str,
        debt_amount: f64,
        debt_asset: &str,
        mode: PositionMode,
        steps: &StrategySwap,
    ) -> Result<u64, soroban_sdk::Error> {
        let debt_decimals = self.resolve_market(debt_asset).decimals;
        let raw_debt = f64_to_i128(debt_amount, debt_decimals);
        let caller_addr = self.get_or_create_user(user);
        let collateral = hub_asset(self.resolve_asset(collateral_asset));
        let debt = hub_asset(self.resolve_asset(debt_asset));

        let ctrl = self.ctrl_client();
        match ctrl.try_multiply(
            &caller_addr,
            &0u64,
            &category,
            &collateral,
            &raw_debt,
            &debt,
            &mode,
            steps,
            &None,
            &None,
        ) {
            Ok(Ok(id)) => {
                let attrs = ctrl.get_account_attributes(&id);
                let user_state = self.users.get_mut(user).expect("user exists");
                user_state.accounts.push(AccountEntry {
                    account_id: id,
                    spoke_id: attrs.spoke_id,
                    mode: attrs.mode,
                });
                if user_state.default_account_id.is_none() {
                    user_state.default_account_id = Some(id);
                }
                Ok(id)
            }
            Ok(Err(err)) => Err(err),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    pub fn try_multiply(
        &mut self,
        user: &str,
        collateral_asset: &str,
        debt_amount: f64,
        debt_asset: &str,
        mode: PositionMode,
        steps: &StrategySwap,
    ) -> Result<u64, soroban_sdk::Error> {
        self.try_multiply_with_category(
            user,
            HARNESS_SPOKE,
            collateral_asset,
            debt_amount,
            debt_asset,
            mode,
            steps,
        )
    }

    pub fn swap_debt(
        &mut self,
        user: &str,
        existing_debt: &str,
        new_amount: f64,
        new_debt: &str,
        steps: &StrategySwap,
    ) {
        self.try_swap_debt(user, existing_debt, new_amount, new_debt, steps)
            .expect("swap_debt");
    }

    pub fn try_swap_debt(
        &mut self,
        user: &str,
        existing_debt: &str,
        new_amount: f64,
        new_debt: &str,
        steps: &StrategySwap,
    ) -> Result<(), soroban_sdk::Error> {
        let account_id = self.try_resolve_account_id(user)?;
        let addr = self.users.get(user).unwrap().address.clone();
        let existing = hub_asset(self.resolve_asset(existing_debt));
        let new = hub_asset(self.resolve_asset(new_debt));
        let decimals = self.resolve_market(new_debt).decimals;
        let raw = f64_to_i128(new_amount, decimals);

        match self
            .ctrl_client()
            .try_swap_debt(&addr, &account_id, &existing, &raw, &new, steps)
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    pub fn swap_collateral(
        &mut self,
        user: &str,
        current_collateral: &str,
        amount: f64,
        new_collateral: &str,
        steps: &StrategySwap,
    ) {
        self.try_swap_collateral(user, current_collateral, amount, new_collateral, steps)
            .expect("swap_collateral");
    }

    pub fn try_swap_collateral(
        &mut self,
        user: &str,
        current_collateral: &str,
        amount: f64,
        new_collateral: &str,
        steps: &StrategySwap,
    ) -> Result<(), soroban_sdk::Error> {
        let account_id = self.try_resolve_account_id(user)?;
        let addr = self.users.get(user).unwrap().address.clone();
        let current = hub_asset(self.resolve_asset(current_collateral));
        let new = hub_asset(self.resolve_asset(new_collateral));
        let decimals = self.resolve_market(current_collateral).decimals;
        let raw = f64_to_i128(amount, decimals);

        match self.ctrl_client().try_swap_collateral(
            &addr,
            &account_id,
            &current,
            &raw,
            &new,
            steps,
        ) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    pub fn repay_debt_with_collateral(
        &mut self,
        user: &str,
        collateral_asset: &str,
        collateral_amount: f64,
        debt_asset: &str,
        steps: &StrategySwap,
        close_position: bool,
    ) {
        self.try_repay_debt_with_collateral(
            user,
            collateral_asset,
            collateral_amount,
            debt_asset,
            steps,
            close_position,
        )
        .expect("repay_debt_with_collateral");
    }

    pub fn try_repay_debt_with_collateral(
        &mut self,
        user: &str,
        collateral_asset: &str,
        collateral_amount: f64,
        debt_asset: &str,
        steps: &StrategySwap,
        close_position: bool,
    ) -> Result<(), soroban_sdk::Error> {
        let account_id = self.try_resolve_account_id(user)?;
        let addr = self.users.get(user).unwrap().address.clone();
        let collateral = hub_asset(self.resolve_asset(collateral_asset));
        let debt = hub_asset(self.resolve_asset(debt_asset));
        let decimals = self.resolve_market(collateral_asset).decimals;
        let raw = f64_to_i128(collateral_amount, decimals);

        match self.ctrl_client().try_repay_debt_with_collateral(
            &addr,
            &account_id,
            &collateral,
            &raw,
            &debt,
            steps,
            &close_position,
        ) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn flash_position(
        &mut self,
        user: &str,
        account_id: u64,
        mode: PositionMode,
        debt_asset: &str,
        debt_amount: f64,
        receiver: &Address,
        data: &Bytes,
        collaterals: &Vec<(controller::types::HubAssetKey, i128)>,
        refund_assets: &Vec<Address>,
    ) -> u64 {
        self.try_flash_position(
            user,
            account_id,
            mode,
            debt_asset,
            debt_amount,
            receiver,
            data,
            collaterals,
            refund_assets,
        )
        .expect("flash_position")
    }

    #[allow(clippy::too_many_arguments)]
    pub fn try_flash_position(
        &mut self,
        user: &str,
        account_id: u64,
        mode: PositionMode,
        debt_asset: &str,
        debt_amount: f64,
        receiver: &Address,
        data: &Bytes,
        collaterals: &Vec<(controller::types::HubAssetKey, i128)>,
        refund_assets: &Vec<Address>,
    ) -> Result<u64, soroban_sdk::Error> {
        let decimals = self.resolve_market(debt_asset).decimals;
        let raw_debt = f64_to_i128(debt_amount, decimals);
        let caller_addr = self.get_or_create_user(user);
        let debt = hub_asset(self.resolve_asset(debt_asset));
        let ctrl = self.ctrl_client();

        match ctrl.try_flash_position(
            &caller_addr,
            &account_id,
            &HARNESS_SPOKE,
            &mode,
            &debt,
            &raw_debt,
            receiver,
            data,
            collaterals,
            refund_assets,
        ) {
            Ok(Ok(id)) => {
                if account_id == 0 {
                    let attrs = ctrl.get_account_attributes(&id);
                    let user_state = self.users.get_mut(user).expect("user exists");
                    user_state.accounts.push(AccountEntry {
                        account_id: id,
                        spoke_id: attrs.spoke_id,
                        mode: attrs.mode,
                    });
                    if user_state.default_account_id.is_none() {
                        user_state.default_account_id = Some(id);
                    }
                }
                Ok(id)
            }
            Ok(Err(err)) => Err(err),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }
}
