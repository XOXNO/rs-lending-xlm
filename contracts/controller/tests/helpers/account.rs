use super::*;
use crate::constants::RAY;
use crate::Controller;
use common::types::PositionManagerConfig;
use soroban_sdk::testutils::Address as _;

fn seed_account(env: &Env, owner: &Address) -> u64 {
    let account_id = 1u64;
    storage::set_account_meta(
        env,
        account_id,
        &AccountMeta {
            owner: owner.clone(),
            spoke_id: 0,
            mode: PositionMode::Normal,
        },
    );
    account_id
}

#[test]
fn owner_passes() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        require_owner_or_delegate(&env, account_id, &owner, &owner);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #44)")]
fn stranger_rejected() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        require_owner_or_delegate(&env, account_id, &Address::generate(&env), &owner);
    });
}

#[test]
fn active_registered_opted_in_delegate_passes() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        let manager = Address::generate(&env);

        storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active: true });
        storage::add_delegate(&env, account_id, &manager);

        require_owner_or_delegate(&env, account_id, &manager, &owner);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #44)")]
fn registered_but_not_opted_in_rejected() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        let manager = Address::generate(&env);

        storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active: true });

        require_owner_or_delegate(&env, account_id, &manager, &owner);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #44)")]
fn opted_in_but_manager_inactive_rejected() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        let manager = Address::generate(&env);

        storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active: false });
        storage::add_delegate(&env, account_id, &manager);

        require_owner_or_delegate(&env, account_id, &manager, &owner);
    });
}

#[test]
fn require_account_owner_owner_passes() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        let meta = require_account_owner(&env, account_id, &owner);
        assert_eq!(meta.owner, owner);
        assert_eq!(meta.spoke_id, 0);
        assert_eq!(meta.mode, PositionMode::Normal);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn require_account_owner_stranger_rejected_as_account_not_in_market() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        let _ = require_account_owner(&env, account_id, &Address::generate(&env));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn require_account_owner_missing_account_rejected_as_account_not_in_market() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let caller = Address::generate(&env);
        let _ = require_account_owner(&env, 99u64, &caller);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn require_account_owner_rejects_active_delegate() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        let manager = Address::generate(&env);

        storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active: true });
        storage::add_delegate(&env, account_id, &manager);

        // Owner-only: delegates must not pass even when fully opted-in/active.
        let _ = require_account_owner(&env, account_id, &manager);
    });
}

#[test]
fn load_existing_account_requires_matching_spoke() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        let mut cache = crate::context::Cache::new_view(&env);
        let (id, account) = load_or_create_account(
            &env,
            &owner,
            account_id,
            0,
            PositionMode::Normal,
            AccountGuard::Supply,
            &mut cache,
        );
        assert_eq!(id, account_id);
        assert_eq!(account.spoke_id, 0);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #310)")]
fn load_existing_account_rejects_spoke_mismatch() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        let mut cache = crate::context::Cache::new_view(&env);
        let _ = load_or_create_account(
            &env,
            &owner,
            account_id,
            1,
            PositionMode::Normal,
            AccountGuard::Supply,
            &mut cache,
        );
    });
}

#[test]
fn update_or_remove_supply_zero_removes_nonzero_keeps() {
    use common::math::fp::{Bps, Ray};
    use common::types::{Account, AccountPosition, HubAssetKey};
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let owner = Address::generate(&env);
        let mut account = Account {
            owner,
            spoke_id: 0,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
        };
        let live = AccountPosition {
            scaled_amount: Ray::from(RAY),
            liquidation_threshold: Bps::from(8_000),
            liquidation_bonus: Bps::from(500),
            loan_to_value: Bps::from(7_500),
            liquidation_fees: Bps::from(100),
        };
        update_or_remove_supply_position(&mut account, &hub, &live);
        assert_eq!(
            account
                .supply_positions
                .get(hub.clone())
                .unwrap()
                .scaled_amount,
            RAY
        );
        let zero = AccountPosition {
            scaled_amount: Ray::ZERO,
            ..live
        };
        update_or_remove_supply_position(&mut account, &hub, &zero);
        assert!(!account.supply_positions.contains_key(hub));
    });
}

#[test]
fn update_or_remove_debt_zero_removes_nonzero_keeps() {
    use common::math::fp::Ray;
    use common::types::{Account, DebtPosition, HubAssetKey};
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let owner = Address::generate(&env);
        let mut account = Account {
            owner,
            spoke_id: 0,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
        };
        update_or_remove_debt_position(
            &mut account,
            &hub,
            &DebtPosition {
                scaled_amount: Ray::from(RAY),
            },
        );
        assert_eq!(
            account
                .borrow_positions
                .get(hub.clone())
                .unwrap()
                .scaled_amount,
            RAY
        );
        update_or_remove_debt_position(
            &mut account,
            &hub,
            &DebtPosition {
                scaled_amount: Ray::ZERO,
            },
        );
        assert!(!account.borrow_positions.contains_key(hub));
    });
}

#[test]
fn cleanup_account_if_empty_removes_only_empty() {
    use common::types::{Account, AccountPositionRaw, HubAssetKey};
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let empty_id = seed_account(&env, &owner);
        let empty = Account {
            owner: owner.clone(),
            spoke_id: 0,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
        };
        cleanup_account_if_empty(&env, &empty, empty_id);
        assert!(storage::try_get_account_meta(&env, empty_id).is_none());

        let live_id = seed_account(&env, &owner);
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut supply = Map::new(&env);
        supply.set(
            hub,
            AccountPositionRaw {
                scaled_amount: RAY,
                liquidation_threshold: 8_000,
                liquidation_bonus: 500,
                loan_to_value: 7_500,
                liquidation_fees: 100,
            },
        );
        let live = Account {
            owner,
            spoke_id: 0,
            mode: PositionMode::Normal,
            supply_positions: supply,
            borrow_positions: Map::new(&env),
        };
        cleanup_account_if_empty(&env, &live, live_id);
        assert!(storage::try_get_account_meta(&env, live_id).is_some());
    });
}
