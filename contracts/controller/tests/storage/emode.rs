use super::*;
use crate::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use crate::Controller;
use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, Map};

// Category reads renew shared-tier TTL once it falls below threshold.
#[test]
fn test_try_get_emode_category_renews_shared_ttl_on_read() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));

    env.as_contract(&contract_id, || {
        let cat = EModeCategoryRaw {
            is_deprecated: false,
            assets: Map::new(&env),
            usage: Map::new(&env),
        };
        set_emode_category(&env, 1, &cat);
        let key = ControllerKey::EModeCategory(1);

        let ttl_after_set = env.storage().persistent().get_ttl(&key);
        let burn = ttl_after_set - TTL_THRESHOLD_SHARED + 1;
        env.ledger().with_mut(|li| li.sequence_number += burn);
        assert!(env.storage().persistent().get_ttl(&key) < TTL_THRESHOLD_SHARED);

        assert!(try_get_emode_category(&env, 1).is_some());

        assert_eq!(
            env.storage().persistent().get_ttl(&key),
            TTL_BUMP_SHARED,
            "read must re-arm the shared bump"
        );
    });
}
