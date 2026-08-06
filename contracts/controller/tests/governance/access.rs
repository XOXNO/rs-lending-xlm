use super::*;
use soroban_sdk::testutils::Address as _;
use stellar_access::access_control::AccessControlStorageKey;
use stellar_access::ownable::OwnableStorageKey;

#[test]
fn init_sets_owner_not_access_control_admin() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin.clone(),));

    env.as_contract(&contract_id, || {
        assert_eq!(ownable::get_owner(&env), Some(admin.clone()));
        assert_eq!(stellar_access::access_control::get_admin(&env), None);
        assert!(!env
            .storage()
            .instance()
            .has(&AccessControlStorageKey::Admin));
        assert!(env.storage().instance().has(&OwnableStorageKey::Owner));
    });
}

#[test]
fn accept_ownership_updates_owner_only() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin.clone(),));
    let client = crate::ControllerClient::new(&env, &contract_id);

    let new_owner = Address::generate(&env);
    let live_until = env.ledger().sequence() + 1_000;
    client.transfer_ownership(&new_owner, &live_until);
    client.accept_ownership();

    env.as_contract(&contract_id, || {
        assert_eq!(ownable::get_owner(&env), Some(new_owner.clone()));
        assert_eq!(stellar_access::access_control::get_admin(&env), None);
    });
}
