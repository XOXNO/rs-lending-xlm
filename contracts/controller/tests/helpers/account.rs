use super::*;
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
