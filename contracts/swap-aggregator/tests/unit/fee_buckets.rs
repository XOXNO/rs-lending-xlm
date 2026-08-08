//! Fee-bucket read-and-clear semantics.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

use crate::types::DataKey;
use crate::Router;

/// Claiming a bucket that holds nothing must not write to storage: the removal
/// would burn rent and ledger footprint for an entry that already reads as
/// zero, and it makes an empty claim indistinguishable from a funded one.
#[test]
fn taking_an_empty_fee_bucket_leaves_its_entry_in_place() {
    let env = Env::default();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let token = Address::generate(&env);

    env.as_contract(&router_addr, || {
        let key = DataKey::AdminFee(token.clone());
        env.storage().persistent().set(&key, &0i128);

        assert_eq!(crate::storage::take_fee_bucket(&env, &key), 0);
        assert!(
            env.storage().persistent().has(&key),
            "an empty bucket must be left untouched, not removed"
        );
    });
}

#[test]
fn taking_a_funded_fee_bucket_clears_it() {
    let env = Env::default();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let token = Address::generate(&env);

    env.as_contract(&router_addr, || {
        let key = DataKey::AdminFee(token.clone());
        env.storage().persistent().set(&key, &7i128);

        assert_eq!(crate::storage::take_fee_bucket(&env, &key), 7);
        assert!(
            !env.storage().persistent().has(&key),
            "a claimed bucket must be removed, not zeroed in place"
        );
        assert_eq!(crate::storage::take_fee_bucket(&env, &key), 0);
    });
}
