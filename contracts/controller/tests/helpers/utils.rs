use super::*;
use common::types::HubAssetKey;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Env, Vec};

#[test]
fn aggregate_payments_dedups_and_preserves_order() {
    let env = Env::default();
    let asset_a = HubAssetKey {
        hub_id: 0,
        asset: Address::generate(&env),
    };
    let asset_b = HubAssetKey {
        hub_id: 0,
        asset: Address::generate(&env),
    };
    let mut payments: Vec<(HubAssetKey, i128)> = Vec::new(&env);
    payments.push_back((asset_a.clone(), 10));
    payments.push_back((asset_a.clone(), 5)); // same asset, summed
    payments.push_back((asset_b.clone(), 3));

    let out = aggregate_positive_payments(&env, &payments);

    assert_eq!(out.len(), 2);
    assert_eq!(out.get_unchecked(0), (asset_a, 15));
    assert_eq!(out.get_unchecked(1), (asset_b, 3));
}

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn aggregate_rejects_negative() {
    let env = Env::default();
    aggregate_payment_amount(&env, None, -1, false);
}

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn aggregate_rejects_zero_when_not_withdraw_all() {
    let env = Env::default();
    aggregate_payment_amount(&env, None, 0, false);
}

#[test]
fn aggregate_zero_is_withdraw_all_sentinel() {
    let env = Env::default();
    assert_eq!(aggregate_payment_amount(&env, None, 0, true), 0);
    assert_eq!(aggregate_payment_amount(&env, Some(0), 5, true), 0);
    assert_eq!(aggregate_payment_amount(&env, None, 5, true), 5);
}

#[test]
fn aggregate_sums_previous_and_amount() {
    let env = Env::default();
    assert_eq!(aggregate_payment_amount(&env, Some(10), 5, false), 15);
    assert_eq!(aggregate_payment_amount(&env, None, 7, false), 7);
    assert_eq!(aggregate_payment_amount(&env, Some(0), 5, false), 5);
}

#[test]
fn push_unique_dedups_preserving_order() {
    let env = Env::default();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let mut out: Vec<Address> = Vec::new(&env);
    push_unique_address(&mut out, a.clone());
    push_unique_address(&mut out, a.clone());
    push_unique_address(&mut out, b.clone());
    assert_eq!(out.len(), 2);
    assert_eq!(out.get_unchecked(0), a);
    assert_eq!(out.get_unchecked(1), b);
}
