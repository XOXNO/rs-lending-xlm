use controller::types::{AccountMeta, ControllerKey, PositionMode, SpokeConfig};

use crate::core::{AccountEntry, LendingTest};

impl LendingTest {
    // Account creation

    /// Create a normal account (e_mode=0, mode=Normal).
    pub fn create_account(&mut self, user: &str) -> u64 {
        let _ = self.get_or_create_user(user);
        let account_id = self.create_account_direct(user, 0, PositionMode::Normal);
        self.register_account(user, account_id, 0, PositionMode::Normal);
        account_id
    }

    /// Create an e-mode account.
    pub fn create_emode_account(&mut self, user: &str, category_id: u32) -> u64 {
        let _ = self.get_or_create_user(user);
        let account_id = self.create_account_direct(user, category_id, PositionMode::Normal);
        self.register_account(user, account_id, category_id, PositionMode::Normal);
        account_id
    }

    /// Create an account with full control over e-mode category and position mode.
    pub fn create_account_full(
        &mut self,
        user: &str,
        e_mode_category: u32,
        mode: PositionMode,
    ) -> u64 {
        let _ = self.get_or_create_user(user);
        let account_id = self.create_account_direct(user, e_mode_category, mode);
        self.register_account(user, account_id, e_mode_category, mode);
        account_id
    }

    pub(crate) fn create_account_direct(
        &self,
        user: &str,
        e_mode_category: u32,
        mode: PositionMode,
    ) -> u64 {
        let owner = self
            .users
            .get(user)
            .map(|state| state.address.clone())
            .unwrap_or_else(|| panic!("user '{}' not found", user));

        self.env.as_contract(&self.controller, || {
            if e_mode_category > 0 {
                let spoke = self
                    .env
                    .storage()
                    .persistent()
                    .get::<_, SpokeConfig>(&ControllerKey::Spoke(e_mode_category))
                    .expect("spoke must exist");
                assert!(!spoke.is_deprecated, "spoke is deprecated");
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
                    spoke_id: e_mode_category,
                    mode,
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
