//! Position add/remove consistency rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::types::AccountPositionType;

#[rule]
fn supply_does_not_decrease_position(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let pos_before = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &asset,
    );

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset.clone(), amount);

    let pos_after = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &asset,
    );

    cvlr_assert!(pos_after >= pos_before);
}

#[rule]
fn borrow_does_not_decrease_debt(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let pos_before = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &asset,
    );

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset.clone(), amount);

    let pos_after = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &asset,
    );

    cvlr_assert!(pos_after >= pos_before);
}

#[rule]
fn withdraw_does_not_increase_position(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let pos_before = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &asset,
    );
    cvlr_assume!(pos_before > 0);

    crate::spec::compat::withdraw_single(e.clone(), caller, account_id, asset.clone(), amount);

    let pos_after = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &asset,
    );

    cvlr_assert!(pos_after <= pos_before);
}

#[rule]
fn repay_does_not_increase_debt(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let pos_before = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &asset,
    );
    cvlr_assume!(pos_before > 0);

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset.clone(), amount);

    let pos_after = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &asset,
    );

    cvlr_assert!(pos_after <= pos_before);
}

#[rule]
fn supply_sanity(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    let amount = crate::constants::WAD;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::compat::supply_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(true);
}
