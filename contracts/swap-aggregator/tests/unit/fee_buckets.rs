//! Fee-bucket read-and-clear semantics.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

use crate::reserved_fee_balance;
use crate::storage::{accumulate_fee, accumulate_swap_fees, take_fee_bucket};
use crate::types::DataKey;
use crate::Router;

/// Taking a zero-balance bucket returns 0 and writes nothing: the entry stays in
/// place.
#[test]
fn taking_an_empty_fee_bucket_leaves_its_entry_in_place() {
    let env = Env::default();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let token = Address::generate(&env);

    env.as_contract(&router_addr, || {
        let key = DataKey::AdminFee(token.clone());
        env.storage().persistent().set(&key, &0i128);

        assert_eq!(take_fee_bucket(&env, &key), 0);
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
        accumulate_fee(&env, key.clone(), 7);

        assert_eq!(take_fee_bucket(&env, &key), 7);
        assert!(
            !env.storage().persistent().has(&key),
            "a claimed bucket must be removed, not zeroed in place"
        );
        assert_eq!(take_fee_bucket(&env, &key), 0);
    });
}

/// Every bucket write moves the token's reserved total by the same amount, in
/// both directions.
///
/// `sweep_balance` trusts `DataKey::ReservedTotal`: a bucket credited without a
/// matching reserve would be swept out as stray balance.
#[test]
fn fee_bucket_writes_move_the_reserved_total_in_step() {
    let env = Env::default();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let token = Address::generate(&env);

    env.as_contract(&router_addr, || {
        let admin_key = DataKey::AdminFee(token.clone());
        let referral_key = DataKey::ReferralFee(4, token.clone());

        accumulate_fee(&env, admin_key.clone(), 10);
        assert_eq!(reserved_fee_balance(&env, &token), 10);

        accumulate_fee(&env, referral_key.clone(), 7);
        accumulate_fee(&env, referral_key.clone(), 3);
        assert_eq!(reserved_fee_balance(&env, &token), 20);

        assert_eq!(take_fee_bucket(&env, &referral_key), 10);
        assert_eq!(reserved_fee_balance(&env, &token), 10);

        assert_eq!(take_fee_bucket(&env, &admin_key), 10);
        assert_eq!(reserved_fee_balance(&env, &token), 0);
        assert!(
            !env.storage()
                .persistent()
                .has(&DataKey::ReservedTotal(token.clone())),
            "a drained reserve must not keep paying rent"
        );
    });
}

/// `accumulate_swap_fees` stores the same state as one `accumulate_fee` call per
/// positive amount: the same bucket balances and reserved total, and no entry for
/// a bucket credited zero.
#[test]
fn combined_swap_accrual_matches_separate_bucket_writes() {
    let env = Env::default();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let both = Address::generate(&env);
    let referral_only = Address::generate(&env);
    let neither = Address::generate(&env);

    env.as_contract(&router_addr, || {
        accumulate_swap_fees(&env, &both, 9, 12, 5);
        let admin_bucket: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::AdminFee(both.clone()))
            .unwrap_or(0);
        let referral_bucket: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::ReferralFee(9, both.clone()))
            .unwrap_or(0);
        assert_eq!(admin_bucket, 12);
        assert_eq!(referral_bucket, 5);
        assert_eq!(reserved_fee_balance(&env, &both), 17);

        // A zero static fee creates no admin bucket but still reserves the
        // referral side.
        accumulate_swap_fees(&env, &referral_only, 3, 0, 4);
        assert!(
            !env.storage()
                .persistent()
                .has(&DataKey::AdminFee(referral_only.clone())),
            "an unfunded bucket must not be written"
        );
        assert_eq!(reserved_fee_balance(&env, &referral_only), 4);

        // Zero on both sides creates no reserve entry.
        accumulate_swap_fees(&env, &neither, 3, 0, 0);
        assert!(
            !env.storage()
                .persistent()
                .has(&DataKey::ReservedTotal(neither.clone())),
            "an empty accrual must not create a reserve entry"
        );
        assert_eq!(reserved_fee_balance(&env, &neither), 0);
    });
}
