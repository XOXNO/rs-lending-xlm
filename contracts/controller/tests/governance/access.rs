use super::*;
use soroban_sdk::testutils::Address as _;
use stellar_access::access_control::AccessControlStorageKey;
use stellar_access::ownable::OwnableStorageKey;

// `sync_pending_admin_transfer` names the *current* admin as the initiator of
// the transfer event, falling back to the owner. With neither configured there
// is no one to name, so it fails closed with `OwnerNotSet` rather than emitting
// an unattributed event.
//
// This is a configuration requirement, not an authorization one: the function
// takes no caller. Caller auth lives on the `transfer_ownership` entrypoint
// that invokes it.
#[test]
#[should_panic(expected = "Error(Contract, #32)")]
fn sync_pending_admin_transfer_panics_when_no_owner_or_admin_configured() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let candidate = Address::generate(&env);
    env.as_contract(&contract_id, || {
        env.storage().instance().remove(&OwnableStorageKey::Owner);
        env.storage()
            .instance()
            .remove(&AccessControlStorageKey::Admin);
        sync_pending_admin_transfer(&env, &candidate, 100);
    });
}

// Accepting ownership must also promote the access-control admin so the new
// owner controls both role systems.
#[test]
fn accept_ownership_promotes_access_control_admin() {
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
        assert_eq!(
            stellar_access::access_control::get_admin(&env),
            Some(new_owner.clone())
        );
        assert_eq!(ownable::get_owner(&env), Some(new_owner.clone()));
    });
}
