use super::*;
use soroban_sdk::testutils::Address as _;

fn new_controller(env: &Env) -> Address {
    let admin = Address::generate(env);
    env.register(Controller, (admin,))
}

// `create_hub` ids start at 1 (hub 0 is seeded in the constructor) and each
// created hub is active on return.
#[test]
fn create_hub_assigns_increasing_ids_and_marks_active() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        let first = create_hub(&env);
        let second = create_hub(&env);
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert!(storage::get_hub(&env, first).is_some_and(|hub| hub.is_active));
        assert!(storage::get_hub(&env, second).is_some_and(|hub| hub.is_active));
    });
}

// Hub 0 is a regular registered hub, seeded active in the constructor exactly
// like every higher id — no implicit/always-active special case.
#[test]
fn require_hub_active_passes_for_registered_hub_zero() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        assert!(storage::get_hub(&env, 0).is_some_and(|hub| hub.is_active));
        require_hub_active(&env, 0);
    });
}

#[test]
fn require_hub_active_passes_for_created_hub() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        let id = create_hub(&env);
        require_hub_active(&env, id);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn require_hub_active_rejects_unknown_hub() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        require_hub_active(&env, 999);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn require_hub_active_rejects_deactivated_hub() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        let id = create_hub(&env);
        storage::set_hub(&env, id, &HubConfig { is_active: false });
        require_hub_active(&env, id);
    });
}
