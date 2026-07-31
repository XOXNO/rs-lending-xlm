use super::*;
use common::types::HubAssetKey;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec};

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
    payments.push_back((asset_a.clone(), 5));
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
    aggregate_payment_amount(&env, None, -1, ZeroLeg::Rejected);
}

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn aggregate_rejects_zero_when_not_withdraw_all() {
    let env = Env::default();
    aggregate_payment_amount(&env, None, 0, ZeroLeg::Rejected);
}

#[test]
fn aggregate_zero_is_withdraw_all_sentinel() {
    let env = Env::default();
    assert_eq!(
        aggregate_payment_amount(&env, None, 0, ZeroLeg::MeansAll),
        0
    );
    assert_eq!(
        aggregate_payment_amount(&env, Some(0), 5, ZeroLeg::MeansAll),
        0
    );
    assert_eq!(
        aggregate_payment_amount(&env, None, 5, ZeroLeg::MeansAll),
        5
    );
}

#[test]
fn aggregate_sums_previous_and_amount() {
    let env = Env::default();
    assert_eq!(
        aggregate_payment_amount(&env, Some(10), 5, ZeroLeg::Rejected),
        15
    );
    assert_eq!(
        aggregate_payment_amount(&env, None, 7, ZeroLeg::Rejected),
        7
    );
    assert_eq!(
        aggregate_payment_amount(&env, Some(0), 5, ZeroLeg::Rejected),
        5
    );
}
