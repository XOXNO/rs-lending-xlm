//! Account isolation (frame) rules.
//!
//! A controller action on one account never mutates another account's
//! positions — cross-account non-interference for supply, borrow, repay and
//! liquidation.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::WAD;
use crate::types::HubAssetKey;
use common::types::Payment;

/// Hub-0 coordinate for `asset`; the spec models the single default hub.
fn hub0(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

fn scaled_supply_at(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let account = crate::storage::get_account(env, account_id);
    account
        .supply_positions
        .get(hub0(asset))
        .map(|p| p.scaled_amount)
        .unwrap_or(0)
}

fn scaled_borrow_at(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let account = crate::storage::get_account(env, account_id);
    account
        .borrow_positions
        .get(hub0(asset))
        .map(|p| p.scaled_amount)
        .unwrap_or(0)
}

#[rule]
fn supply_does_not_change_other_account_positions(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let target_account: u64 = 1;
    let other_account: u64 = 2;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let other_supply_before = scaled_supply_at(&e, other_account, &asset);
    let other_borrow_before = scaled_borrow_at(&e, other_account, &asset);

    crate::spec::compat::supply_single(e.clone(), caller, target_account, asset.clone(), amount);

    cvlr_assert!(scaled_supply_at(&e, other_account, &asset) == other_supply_before);
    cvlr_assert!(scaled_borrow_at(&e, other_account, &asset) == other_borrow_before);
}

#[rule]
fn borrow_does_not_change_other_account_positions(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let target_account: u64 = 1;
    let other_account: u64 = 2;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let other_supply_before = scaled_supply_at(&e, other_account, &asset);
    let other_borrow_before = scaled_borrow_at(&e, other_account, &asset);

    crate::spec::compat::borrow_single(e.clone(), caller, target_account, asset.clone(), amount);

    cvlr_assert!(scaled_supply_at(&e, other_account, &asset) == other_supply_before);
    cvlr_assert!(scaled_borrow_at(&e, other_account, &asset) == other_borrow_before);
}

#[rule]
fn repay_only_changes_target_account_debt(e: Env, caller: Address, asset: Address, amount: i128) {
    let target_account: u64 = 1;
    let other_account: u64 = 2;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let other_supply_before = scaled_supply_at(&e, other_account, &asset);
    let other_borrow_before = scaled_borrow_at(&e, other_account, &asset);

    crate::spec::compat::repay_single(e.clone(), caller, target_account, asset.clone(), amount);

    cvlr_assert!(scaled_supply_at(&e, other_account, &asset) == other_supply_before);
    cvlr_assert!(scaled_borrow_at(&e, other_account, &asset) == other_borrow_before);
}

/// Liquidating one account never mutates another account's positions.
/// Audits the repaid debt asset, the asset most exposed to a buggy
/// cross-account write.
#[rule]
fn liquidation_does_not_change_other_account_positions(
    e: Env,
    liquidator: Address,
    debt_asset: Address,
    debt_amount: i128,
) {
    let target_account: u64 = 1;
    let other_account: u64 = 2;
    cvlr_assume!(debt_amount > 0 && debt_amount <= WAD * 1000);

    let other_supply_before = scaled_supply_at(&e, other_account, &debt_asset);
    let other_borrow_before = scaled_borrow_at(&e, other_account, &debt_asset);

    let mut payments: soroban_sdk::Vec<Payment> = soroban_sdk::Vec::new(&e);
    payments.push_back((debt_asset.clone(), debt_amount));
    crate::spec::compat::liquidate(e.clone(), liquidator, target_account, payments);

    cvlr_assert!(scaled_supply_at(&e, other_account, &debt_asset) == other_supply_before);
    cvlr_assert!(scaled_borrow_at(&e, other_account, &debt_asset) == other_borrow_before);
}

#[rule]
fn account_isolation_reachability(e: Env, caller: Address, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);
    crate::spec::compat::supply_single(e, caller, 1, asset, amount);
    cvlr_satisfy!(true);
}
