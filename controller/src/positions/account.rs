use common::types::{Account, PositionMode};
use soroban_sdk::{Address, Env, Map};

use super::emode;
use crate::storage;

pub fn create_account(
    env: &Env,
    owner: &Address,
    e_mode_category: u32,
    mode: PositionMode,
    is_isolated: bool,
    isolated_asset: Option<Address>,
) -> u64 {
    // Validate: e-mode and isolation are mutually exclusive
    emode::validate_e_mode_isolation_exclusion(env, e_mode_category, is_isolated);

    // Validate: e-mode category must not be deprecated
    let emode_cat = emode::e_mode_category(env, e_mode_category);
    emode::ensure_e_mode_not_deprecated(env, &emode_cat);

    let account_id = storage::increment_account_nonce(env);
    let account = Account {
        owner: owner.clone(),
        is_isolated,
        e_mode_category_id: e_mode_category,
        mode,
        isolated_asset,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    };
    storage::set_account(env, account_id, &account);

    account_id
}

pub fn remove_account(env: &Env, account_id: u64) {
    storage::remove_account_entry(env, account_id);
}

pub fn cleanup_account_if_empty(env: &Env, account: &Account, account_id: u64) {
    if account.supply_positions.is_empty() && account.borrow_positions.is_empty() {
        remove_account(env, account_id);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::types::{AccountPosition, PositionMode};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, Map};

    struct TestSetup {
        env: Env,
        contract: Address,
        owner: Address,
        asset: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let contract = env.register(crate::Controller, (admin,));
            let owner = Address::generate(&env);
            let asset = Address::generate(&env);

            Self {
                env,
                contract,
                owner,
                asset,
            }
        }

        fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.contract, f)
        }

        fn non_empty_account(&self) -> Account {
            let mut supply_positions = Map::new(&self.env);
            supply_positions.set(
                self.asset.clone(),
                AccountPosition {
                    position_type: common::types::AccountPositionType::Deposit,
                    asset: self.asset.clone(),
                    scaled_amount_ray: 123,
                    account_id: 1,
                    liquidation_threshold_bps: 8_000,
                    liquidation_bonus_bps: 500,
                    liquidation_fees_bps: 100,
                    loan_to_value_bps: 7_500,
                },
            );

            Account {
                owner: self.owner.clone(),
                is_isolated: false,
                e_mode_category_id: 0,
                mode: common::types::PositionMode::Normal,
                isolated_asset: None,
                supply_positions,
                borrow_positions: Map::new(&self.env),
            }
        }
    }

    #[test]
    fn test_create_account_persists_state_and_increments_nonce() {
        let t = TestSetup::new();

        t.as_contract(|| {
            let id = create_account(&t.env, &t.owner, 0, PositionMode::Long, false, None);
            let account = storage::get_account(&t.env, id);

            assert_eq!(id, 1);
            assert_eq!(account.owner, t.owner);
            assert!(!account.is_isolated);
            assert_eq!(account.mode, common::types::PositionMode::Long);
            assert_eq!(storage::get_account_nonce(&t.env), 1);
        });
    }

    #[test]
    fn test_remove_account_deletes_storage_entry() {
        let t = TestSetup::new();

        t.as_contract(|| {
            let id = create_account(&t.env, &t.owner, 0, PositionMode::Normal, false, None);
            assert!(storage::try_get_account(&t.env, id).is_some());

            remove_account(&t.env, id);

            assert!(storage::try_get_account(&t.env, id).is_none());
        });
    }

    #[test]
    fn test_cleanup_account_if_empty_only_removes_empty_accounts() {
        let t = TestSetup::new();

        t.as_contract(|| {
            let empty_id = create_account(&t.env, &t.owner, 0, PositionMode::Normal, false, None);
            let empty_account = storage::get_account(&t.env, empty_id);
            cleanup_account_if_empty(&t.env, &empty_account, empty_id);
            assert!(storage::try_get_account(&t.env, empty_id).is_none());

            let non_empty_id =
                create_account(&t.env, &t.owner, 0, PositionMode::Normal, false, None);
            let non_empty_account = t.non_empty_account();
            storage::set_account(&t.env, non_empty_id, &non_empty_account);
            cleanup_account_if_empty(&t.env, &non_empty_account, non_empty_id);
            assert!(storage::try_get_account(&t.env, non_empty_id).is_some());
        });
    }
}
