use super::*;
use crate::Controller;
use controller_interface::types::PositionManagerConfig;
use soroban_sdk::testutils::Address as _;

/// Seeds account `1` owned by `owner` and returns its id.
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

// The owner always passes, regardless of delegate/manager state.
#[test]
fn owner_passes() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        require_owner_or_delegate(&env, account_id, &owner);
    });
}

// A non-owner with no delegation is rejected (owner-only behavior preserved).
#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn stranger_rejected() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        require_owner_or_delegate(&env, account_id, &Address::generate(&env));
    });
}

// A registered, active manager the owner opted in passes.
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

        require_owner_or_delegate(&env, account_id, &manager);
    });
}

// A registered, active manager NOT opted into the account is rejected.
#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn registered_but_not_opted_in_rejected() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        let account_id = seed_account(&env, &owner);
        let manager = Address::generate(&env);

        storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active: true });

        require_owner_or_delegate(&env, account_id, &manager);
    });
}

// An opted-in delegate whose manager registration is inactive is rejected.
#[test]
#[should_panic(expected = "Error(Contract, #13)")]
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

        require_owner_or_delegate(&env, account_id, &manager);
    });
}
