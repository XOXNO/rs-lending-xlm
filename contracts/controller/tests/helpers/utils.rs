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
    aggregate_payment_amount(&env, None, -1);
}

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn aggregate_rejects_zero() {
    let env = Env::default();
    aggregate_payment_amount(&env, None, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn aggregate_rejects_overflow() {
    let env = Env::default();
    aggregate_payment_amount(&env, Some(i128::MAX), 1);
}

#[test]
fn aggregate_sums_previous_and_amount() {
    let env = Env::default();
    assert_eq!(aggregate_payment_amount(&env, Some(10), 5), 15);
    assert_eq!(aggregate_payment_amount(&env, None, 7), 7);
    assert_eq!(aggregate_payment_amount(&env, None, i128::MAX), i128::MAX);
}

#[test]
fn balance_snapshot_reads_a_duplicate_asset_only_once() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let holder = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(admin).address();
    soroban_sdk::token::StellarAssetClient::new(&env, &asset).mint(&holder, &19);

    let balances = snapshot_balances(&env, &holder, [asset.clone(), asset.clone()]);

    assert_eq!(balances.len(), 1);
    assert_eq!(balances.get(asset), Some(19));
}
