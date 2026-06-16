use controller::types::{AccountMeta, ControllerKey, EModeCategoryRaw, PositionMode};
use soroban_sdk::Address;

use crate::core::{AccountEntry, LendingTest};

impl LendingTest {
    // Account creation

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

    pub(crate) fn create_account_direct(
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
                    .get::<_, EModeCategoryRaw>(&ControllerKey::EModeCategory(e_mode_category))
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

    pub(crate) fn register_account(
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
        if default_is_missing {
            user_state.default_account_id = Some(account_id);
        }
    }

    // Account removal

    /// Remove an account (must have no positions).
    pub fn remove_account(&mut self, user: &str) {
        let account_id = self.resolve_account_id(user);
        self.remove_account_direct(account_id)
            .expect("remove should succeed");

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

    pub(crate) fn remove_account_direct(&self, account_id: u64) -> Result<(), soroban_sdk::Error> {
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
}
