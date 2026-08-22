use controller::types::{AccountMeta, ControllerKey, PositionMode, SpokeConfig};

use crate::core::{AccountEntry, LendingTest};
use crate::helpers::HARNESS_SPOKE;

impl LendingTest {
    pub fn create_account(&mut self, user: &str) -> u64 {
        self.create_account_full(user, HARNESS_SPOKE, PositionMode::Normal)
    }

    pub fn create_spoke_account(&mut self, user: &str, category_id: u32) -> u64 {
        self.create_account_full(user, category_id, PositionMode::Normal)
    }

    pub fn create_account_full(&mut self, user: &str, spoke_id: u32, mode: PositionMode) -> u64 {
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
            let spoke = self
                .env
                .storage()
                .persistent()
                .get::<_, SpokeConfig>(&ControllerKey::Spoke(spoke_id))
                .expect("spoke must exist");
            assert!(!spoke.is_deprecated, "spoke is deprecated");
        });

        let token_id =
            position_nft::PositionNftClient::new(&self.env, &self.position_nft).mint(&owner);
        let account_id = u64::from(token_id);

        self.env.as_contract(&self.controller, || {
            self.env.storage().persistent().set(
                &ControllerKey::AccountMeta(account_id),
                &AccountMeta { spoke_id, mode },
            );
        });

        account_id
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

    pub fn enable_delegate(&mut self, owner: &str, delegate: &str, account_id: u64) {
        let owner_addr = self.get_or_create_user(owner);
        let delegate_addr = self.get_or_create_user(delegate);
        let ctrl = self.ctrl_client();
        ctrl.set_position_manager(&delegate_addr, &true);
        ctrl.add_delegate(&owner_addr, &account_id, &delegate_addr);
    }

    pub fn remove_account(&mut self, user: &str) {
        let account_id = self.resolve_account_id(user);
        self.remove_account_direct(account_id)
            .expect("remove should succeed");

        let user_state = self.users.get_mut(user).unwrap();
        user_state.accounts.retain(|a| a.account_id != account_id);
        user_state.default_account_id = user_state.accounts.first().map(|a| a.account_id);
    }

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
        })?;

        position_nft::PositionNftClient::new(&self.env, &self.position_nft)
            .burn(&u32::try_from(account_id).expect("test account ids fit u32"));
        Ok(())
    }

    pub fn nft_owner_of(&self, account_id: u64) -> soroban_sdk::Address {
        position_nft::PositionNftClient::new(&self.env, &self.position_nft)
            .owner_of(&u32::try_from(account_id).expect("test account ids fit u32"))
    }

    pub fn try_nft_owner_of(&self, account_id: u64) -> bool {
        position_nft::PositionNftClient::new(&self.env, &self.position_nft)
            .try_owner_of(&u32::try_from(account_id).expect("test account ids fit u32"))
            .is_ok()
    }

    pub fn nft_transfer(&mut self, from: &str, to: &str, account_id: u64) {
        let from_addr = self.get_or_create_user(from);
        let to_addr = self.get_or_create_user(to);
        position_nft::PositionNftClient::new(&self.env, &self.position_nft).transfer(
            &from_addr,
            &to_addr,
            &u32::try_from(account_id).expect("test account ids fit u32"),
        );
    }

    /// Post-transfer bookkeeping: registers `account_id` against `user` in the
    /// harness's local user index so subsequent harness verbs keyed by user
    /// name (`supply`, `borrow`, `resolve_account_id`, ...) resolve to the
    /// account the NFT was just transferred to. Does not touch on-chain
    /// state -- the NFT transfer itself (via `nft_transfer`) is the source of
    /// truth for ownership.
    ///
    /// Also prunes `account_id` from every *other* user's bookkeeping --
    /// mirroring what `remove_account` does for the id it removes -- so a
    /// stale entry doesn't linger in the old owner's `accounts`/
    /// `default_account_id` and cause harness verbs invoked as the old owner
    /// to keep resolving an account they no longer own.
    pub fn adopt_account(
        &mut self,
        user: &str,
        account_id: u64,
        spoke_id: u32,
        mode: PositionMode,
    ) {
        let _ = self.get_or_create_user(user);
        for (name, state) in self.users.iter_mut() {
            if name.as_str() == user {
                continue;
            }
            let had_it = state.accounts.iter().any(|a| a.account_id == account_id);
            if !had_it {
                continue;
            }
            state.accounts.retain(|a| a.account_id != account_id);
            state.default_account_id = state.accounts.first().map(|a| a.account_id);
        }
        self.register_account(user, account_id, spoke_id, mode);
    }
}
