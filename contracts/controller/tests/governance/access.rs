use super::*;
use soroban_sdk::testutils::Address as _;
use stellar_access::access_control::AccessControlStorageKey;
use stellar_access::ownable::OwnableStorageKey;

#[test]
#[should_panic(expected = "Error(Contract, #32)")]
fn sync_pending_admin_transfer_requires_owner_or_admin() {
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
