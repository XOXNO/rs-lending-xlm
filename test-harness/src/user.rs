use common::types::{AccountMeta, ControllerKey, EModeCategory, PositionMode};
use soroban_sdk::{vec, Address, Vec};

use crate::context::{AccountEntry, LendingTest};
use crate::helpers::f64_to_i128;

impl LendingTest {
    // -----------------------------------------------------------------------
    // Account creation
    // -----------------------------------------------------------------------

    /// Create a normal account (e_mode=0, mode=Normal, not isolated).
    pub fn create_account(&mut self, user: &str) -> u64 {
        let _ = self.get_or_create_user(user);
        let account_id = self.create_account_direct(user, 0, PositionMode::Normal, false, None);
        self.register_account(user, account_id, 0, PositionMode::Normal, false);
        account_id
    }

    /// Create an e-mode account.
    pub fn create_emode_account(&mut self, user: &str, category_id: u32) -> u64 {
        let _ = self.get_or_create_user(user);
        let account_id =
            self.create_account_direct(user, category_id, PositionMode::Normal, false, None);
        self.register_account(user, account_id, category_id, PositionMode::Normal, false);
        account_id
    }

    /// Create an isolated account.
    pub fn create_isolated_account(&mut self, user: &str, asset_name: &str) -> u64 {
        let _ = self.get_or_create_user(user);
        let asset_addr = self.resolve_asset(asset_name);
        let account_id =
            self.create_account_direct(user, 0, PositionMode::Normal, true, Some(asset_addr));
        self.register_account(user, account_id, 0, PositionMode::Normal, true);
        account_id
    }

    /// Create an account with full control over all parameters.
    pub fn create_account_full(
        &mut self,
        user: &str,
        e_mode_category: u32,
        mode: PositionMode,
        is_isolated: bool,
    ) -> u64 {
        let _ = self.get_or_create_user(user);
        let account_id = self.create_account_direct(user, e_mode_category, mode, is_isolated, None);
        self.register_account(user, account_id, e_mode_category, mode, is_isolated);
        account_id
    }

    fn create_account_direct(
        &self,
        user: &str,
        e_mode_category: u32,
        mode: PositionMode,
        is_isolated: bool,
        isolated_asset: Option<Address>,
    ) -> u64 {
        let owner = self
            .users
            .get(user)
            .map(|state| state.address.clone())
            .unwrap_or_else(|| panic!("user '{}' not found", user));

        self.env.as_contract(&self.controller, || {
            assert!(
                !(e_mode_category > 0 && is_isolated),
                "e-mode and isolation are mutually exclusive"
            );

            if e_mode_category > 0 {
                let category = self
                    .env
                    .storage()
                    .persistent()
                    .get::<_, EModeCategory>(&ControllerKey::EModeCategory(e_mode_category))
                    .expect("e-mode category must exist");
                assert!(!category.is_deprecated, "e-mode category is deprecated");
            }

            let current = self
                .env
                .storage()
                .instance()
                .get::<_, u64>(&ControllerKey::AccountNonce)
                .unwrap_or(0);
            let next = current + 1;
            self.env
                .storage()
                .instance()
                .set(&ControllerKey::AccountNonce, &next);

            self.env.storage().persistent().set(
                &ControllerKey::AccountMeta(next),
                &AccountMeta {
                    owner,
                    is_isolated,
                    e_mode_category_id: e_mode_category,
                    mode,
                    isolated_asset,
                },
            );
            next
        })
    }

    fn register_account(
        &mut self,
        user: &str,
        account_id: u64,
        e_mode_category: u32,
        mode: PositionMode,
        is_isolated: bool,
    ) {
        let default_is_missing = self
            .users
            .get(user)
            .and_then(|state| state.default_account_id)
            .is_none_or(|existing| !self.account_exists(existing));

        let user_state = self.users.get_mut(user).expect("user must exist");
        user_state.accounts.push(AccountEntry {
            account_id,
            e_mode_category,
            mode,
            is_isolated,
        });
        // Set the default account when none exists or the previous default
        // was already removed by contract-side cleanup.
        if default_is_missing {
            user_state.default_account_id = Some(account_id);
        }
    }

    // -----------------------------------------------------------------------
    // Supply
    // -----------------------------------------------------------------------

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

        // Auto-mint tokens to user
        market.token_admin.mint(&addr, &amount);

        // Determine account_id (0 = create new if no default)
        let account_id = self.default_account_id_or_zero(user);

        let ctrl = self.ctrl_client();
        let assets: Vec<(Address, i128)> = vec![&self.env, (asset_addr, amount)];
        let returned_id = ctrl.supply(&addr, &account_id, &0u32, &assets);

        // Register account if newly created
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
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let addr = self.get_or_create_user(user);
        let market = self.resolve_market(asset_name);
        let asset_addr = market.asset.clone();
        market.token_admin.mint(&addr, &raw_amount);

        let account_id = self.default_account_id_or_zero(user);

        let ctrl = self.ctrl_client();
        let assets: Vec<(Address, i128)> = vec![&self.env, (asset_addr, raw_amount)];
        match ctrl.try_supply(&addr, &account_id, &0u32, &assets) {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(err)) => Err(err),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    // -----------------------------------------------------------------------
    // Supply bulk
    // -----------------------------------------------------------------------

    /// Supply multiple assets in a single controller call.
    /// Auto-mints tokens for each asset. Auto-creates account if needed.
    ///
    /// ```rust
    /// # use test_harness::{LendingTest, ALICE, eth_preset, usdc_preset};
    /// # let mut ctx = LendingTest::new().with_market(eth_preset()).with_market(usdc_preset()).build();
    /// ctx.supply_bulk(ALICE, &[("USDC", 10_000.0), ("ETH", 5.0)]);
    /// ```
    pub fn supply_bulk(&mut self, user: &str, assets: &[(&str, f64)]) {
        let addr = self.get_or_create_user(user);

        // Mint tokens + build Vec
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

    // -----------------------------------------------------------------------
    // Borrow
    // -----------------------------------------------------------------------

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
        let borrows: Vec<(Address, i128)> = vec![&self.env, (asset_addr, amount)];
        ctrl.borrow(&addr, &account_id, &borrows);
    }

    pub fn borrow_to(&mut self, user: &str, account_id: u64, asset_name: &str, amount: f64) {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let addr = self.users.get(user).unwrap().address.clone();
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        let borrows: Vec<(Address, i128)> = vec![&self.env, (asset_addr, raw_amount)];
        ctrl.borrow(&addr, &account_id, &borrows);
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
        let borrows: Vec<(Address, i128)> = vec![&self.env, (asset_addr, raw_amount)];
        match ctrl.try_borrow(&addr, &account_id, &borrows) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    // -----------------------------------------------------------------------
    // Borrow bulk
    // -----------------------------------------------------------------------

    /// Borrow multiple assets in a single controller call.
    /// HF check runs once AFTER all borrows (cumulative).
    ///
    /// ```rust
    /// # use test_harness::{LendingTest, ALICE, eth_preset, wbtc_preset};
    /// # let mut ctx = LendingTest::new().with_market(eth_preset()).with_market(wbtc_preset()).build();
    /// # ctx.supply(ALICE, "ETH", 10.0);
    /// ctx.borrow_bulk(ALICE, &[("ETH", 1.0), ("WBTC", 0.01)]);
    /// ```
    pub fn borrow_bulk(&mut self, user: &str, assets: &[(&str, f64)]) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();

        let mut soroban_borrows: Vec<(Address, i128)> = Vec::new(&self.env);
        for (asset_name, amount) in assets {
            let market = self.resolve_market(asset_name);
            let raw = f64_to_i128(*amount, market.decimals);
            soroban_borrows.push_back((market.asset.clone(), raw));
        }

        let ctrl = self.ctrl_client();
        ctrl.borrow(&addr, &account_id, &soroban_borrows);
    }

    pub fn try_borrow_bulk(
        &mut self,
        user: &str,
        assets: &[(&str, f64)],
    ) -> Result<(), soroban_sdk::Error> {
        let account_id = self.try_resolve_account_id(user)?;
        let addr = self.users.get(user).unwrap().address.clone();

        let mut soroban_borrows: Vec<(Address, i128)> = Vec::new(&self.env);
        for (asset_name, amount) in assets {
            let market = self.resolve_market(asset_name);
            let raw = f64_to_i128(*amount, market.decimals);
            soroban_borrows.push_back((market.asset.clone(), raw));
        }

        let ctrl = self.ctrl_client();
        match ctrl.try_borrow(&addr, &account_id, &soroban_borrows) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    // -----------------------------------------------------------------------
    // Withdraw
    // -----------------------------------------------------------------------

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
        let withdrawals: Vec<(Address, i128)> = vec![&self.env, (asset_addr, amount)];
        ctrl.withdraw(&addr, &account_id, &withdrawals);
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
        let withdrawals: Vec<(Address, i128)> = vec![&self.env, (asset_addr, raw_amount)];
        match ctrl.try_withdraw(&addr, &account_id, &withdrawals) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    // -----------------------------------------------------------------------
    // Withdraw bulk
    // -----------------------------------------------------------------------

    /// Withdraw multiple assets in a single controller call.
    /// HF check runs once AFTER all withdrawals (if borrows exist).
    ///
    /// ```rust
    /// # use test_harness::{LendingTest, ALICE, eth_preset, usdc_preset};
    /// # let mut ctx = LendingTest::new().with_market(eth_preset()).with_market(usdc_preset()).build();
    /// # ctx.supply(ALICE, "USDC", 10000.0);
    /// # ctx.supply(ALICE, "ETH", 1.0);
    /// ctx.withdraw_bulk(ALICE, &[("USDC", 1_000.0), ("ETH", 0.5)]);
    /// ```
    pub fn withdraw_bulk(&mut self, user: &str, assets: &[(&str, f64)]) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();

        let mut soroban_withdrawals: Vec<(Address, i128)> = Vec::new(&self.env);
        for (asset_name, amount) in assets {
            let market = self.resolve_market(asset_name);
            let raw = f64_to_i128(*amount, market.decimals);
            soroban_withdrawals.push_back((market.asset.clone(), raw));
        }

        let ctrl = self.ctrl_client();
        ctrl.withdraw(&addr, &account_id, &soroban_withdrawals);
    }

    // -----------------------------------------------------------------------
    // Repay
    // -----------------------------------------------------------------------

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

        // Auto-mint tokens for repayment
        market.token_admin.mint(&addr, &amount);

        let ctrl = self.ctrl_client();
        let payments: Vec<(Address, i128)> = vec![&self.env, (asset_addr, amount)];
        ctrl.repay(&addr, &account_id, &payments);
    }

    // -----------------------------------------------------------------------
    // Repay bulk
    // -----------------------------------------------------------------------

    /// Repay multiple assets in a single controller call.
    /// Auto-mints tokens for each repayment.
    ///
    /// ```rust
    /// # use test_harness::{LendingTest, ALICE, eth_preset, wbtc_preset};
    /// # let mut ctx = LendingTest::new().with_market(eth_preset()).with_market(wbtc_preset()).build();
    /// # ctx.supply(ALICE, "ETH", 10.0);
    /// # ctx.borrow(ALICE, "ETH", 1.0);
    /// # ctx.borrow(ALICE, "WBTC", 0.01);
    /// ctx.repay_bulk(ALICE, &[("ETH", 0.5), ("WBTC", 0.005)]);
    /// ```
    pub fn repay_bulk(&mut self, user: &str, assets: &[(&str, f64)]) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();

        let mut soroban_payments: Vec<(Address, i128)> = Vec::new(&self.env);
        for (asset_name, amount) in assets {
            let market = self.resolve_market(asset_name);
            let raw = f64_to_i128(*amount, market.decimals);
            market.token_admin.mint(&addr, &raw);
            soroban_payments.push_back((market.asset.clone(), raw));
        }

        let ctrl = self.ctrl_client();
        ctrl.repay(&addr, &account_id, &soroban_payments);
    }

    // -----------------------------------------------------------------------
    // Account removal
    // -----------------------------------------------------------------------

    /// Remove an account (must have no positions).
    pub fn remove_account(&mut self, user: &str) {
        let account_id = self.resolve_account_id(user);
        self.remove_account_direct(account_id)
            .expect("remove should succeed");

        // Update internal state
        let user_state = self.users.get_mut(user).unwrap();
        user_state.accounts.retain(|a| a.account_id != account_id);
        user_state.default_account_id = user_state.accounts.first().map(|a| a.account_id);
    }

    /// Remove a specific account by ID.
    pub fn remove_account_by_id(&mut self, user: &str, account_id: u64) {
        self.remove_account_direct(account_id)
            .expect("remove should succeed");

        let user_state = self.users.get_mut(user).unwrap();
        user_state.accounts.retain(|a| a.account_id != account_id);
        if user_state.default_account_id == Some(account_id) {
            user_state.default_account_id = user_state.accounts.first().map(|a| a.account_id);
        }
    }

    /// Try to remove an account -- returns Result.
    pub fn try_remove_account(&mut self, user: &str) -> Result<(), soroban_sdk::Error> {
        let account_id = self.try_resolve_account_id(user)?;
        match self.remove_account_direct(account_id) {
            Ok(()) => {
                let user_state = self.users.get_mut(user).unwrap();
                user_state.accounts.retain(|a| a.account_id != account_id);
                user_state.default_account_id = user_state.accounts.first().map(|a| a.account_id);
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    fn remove_account_direct(&self, account_id: u64) -> Result<(), soroban_sdk::Error> {
        self.env.as_contract(&self.controller, || {
            let persistent = self.env.storage().persistent();
            if !persistent.has(&ControllerKey::AccountMeta(account_id)) {
                return Err(soroban_sdk::Error::from_contract_error(
                    common::errors::GenericError::AccountNotFound as u32,
                ));
            };

            let has_supply = persistent.has(&ControllerKey::SupplyPositions(account_id));
            let has_borrow = persistent.has(&ControllerKey::BorrowPositions(account_id));
            if has_supply || has_borrow {
                return Err(soroban_sdk::Error::from_contract_error(
                    common::errors::CollateralError::PositionNotFound as u32,
                ));
            }

            persistent.remove(&ControllerKey::AccountMeta(account_id));
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Withdraw all
    // -----------------------------------------------------------------------

    /// Withdraw the entire position for an asset (passes amount=0 which means "withdraw all").
    pub fn withdraw_all(&mut self, user: &str, asset_name: &str) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        let withdrawals: Vec<(Address, i128)> = vec![&self.env, (asset_addr, 0i128)];
        ctrl.withdraw(&addr, &account_id, &withdrawals);
    }

    // -----------------------------------------------------------------------
    // Repay (continued)
    // -----------------------------------------------------------------------

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
        let payments: Vec<(Address, i128)> = vec![&self.env, (asset_addr, raw_amount)];
        match ctrl.try_repay(&addr, &account_id, &payments) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }
}
