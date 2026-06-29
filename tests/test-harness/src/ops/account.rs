use controller::types::{AccountMeta, ControllerKey, PositionMode, SpokeConfig};

use crate::core::{AccountEntry, LendingTest};
use crate::helpers::HARNESS_SPOKE;

impl LendingTest {
    // Account creation

    /// Create a normal account on the base harness spoke (mode=Normal).
    pub fn create_account(&mut self, user: &str) -> u64 {
        let _ = self.get_or_create_user(user);
        let account_id = self.create_account_direct(user, HARNESS_SPOKE, PositionMode::Normal);
        self.register_account(user, account_id, HARNESS_SPOKE, PositionMode::Normal);
        account_id
    }

    /// Create an account on spoke `category_id` (id >= 2).
    pub fn create_spoke_account(&mut self, user: &str, category_id: u32) -> u64 {
        let _ = self.get_or_create_user(user);
        let account_id = self.create_account_direct(user, category_id, PositionMode::Normal);
        self.register_account(user, account_id, category_id, PositionMode::Normal);
        account_id
    }

    /// Create an account with full control over the spoke and position mode.
    /// `spoke_id` is the target spoke id (>= 1); use [`HARNESS_SPOKE`] for
    /// a regular account.
    pub fn create_account_full(
        &mut self,
        user: &str,
        spoke_id: u32,
        mode: PositionMode,
    ) -> u64 {
        let _ = self.get_or_create_user(user);
        let account_id = self.create_account_direct(user, spoke_id, mode);
        self.register_account(user, account_id, spoke_id, mode);
        account_id
    }

    pub(crate) fn create_account_direct(
        &self,
        user: &str,
        spoke_id: u32,
        mode: PositionMode,
    ) -> u64 {
        let owner = self
            .users
            .get(user)
            .map(|state| state.address.clone())
            .unwrap_or_else(|| panic!("user '{}' not found", user));

        self.env.as_contract(&self.controller, || {
            // Every account binds to a real spoke (id >= 1); there is no spoke 0.
            let spoke = self
                .env
                .storage()
                .persistent()
                .get::<_, SpokeConfig>(&ControllerKey::Spoke(spoke_id))
                .expect("spoke must exist");
            assert!(!spoke.is_deprecated, "spoke is deprecated");

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
                    spoke_id,
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
        spoke_id: u32,
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
            spoke_id,
            mode,
        });
        if default_is_missing {
            user_state.default_account_id = Some(account_id);
        }
    }

    /// Registers `delegate` as an active position manager and opts it into
    /// `account_id` on `owner`'s behalf -- the two owner-gated steps a real
    /// delegate setup requires before it can act on the account.
    pub fn enable_delegate(&mut self, owner: &str, delegate: &str, account_id: u64) {
        let owner_addr = self.get_or_create_user(owner);
        let delegate_addr = self.get_or_create_user(delegate);
        let ctrl = self.ctrl_client();
        ctrl.set_position_manager(&delegate_addr, &true);
        ctrl.add_delegate(&owner_addr, &account_id, &delegate_addr);
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
