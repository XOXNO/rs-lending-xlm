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

/// The constructor's state must be reachable from the event stream alone.
///
/// `ownable::set_owner` and the storage setters are silent writes, so before
/// these emissions an indexer replaying from genesis could never learn the
/// owner, the position limits or the borrow-collateral floor — they only
/// became observable once someone changed them.
#[test]
fn init_emits_owner_and_default_limits() {
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::xdr::{ContractEventBody, ScSymbol, ScVal};

    let env = Env::default();
    let admin = Address::generate(&env);
    env.register(Controller, (admin.clone(),));

    let symbol = |name: &str| ScVal::Symbol(ScSymbol(name.try_into().unwrap()));
    let mut saw_owner = false;
    let mut saw_limits = false;
    let mut saw_floor = false;

    for event in env.events().all().events().iter() {
        let ContractEventBody::V0(body) = &event.body;
        let topics = body.topics.as_slice();

        if topics.first() == Some(&symbol("ownership_transfer_completed")) {
            saw_owner = true;
        }
        if topics.get(1) == Some(&symbol("position_limits")) {
            saw_limits = true;
        }
        if topics.get(1) == Some(&symbol("min_borrow_collateral")) {
            saw_floor = true;
        }
    }

    assert!(saw_owner, "constructor must publish the initial owner");
    assert!(saw_limits, "constructor must publish the default position limits");
    assert!(saw_floor, "constructor must publish the default borrow floor");
}
