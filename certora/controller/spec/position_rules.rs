//! Position add/remove consistency rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::types::AccountPositionType;

/// Supply increases the account's deposit scaled amount.
#[rule]
fn supply_increases_position(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);

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

    cvlr_assert!(pos_after > pos_before);
}

/// Borrow increases the account's debt scaled amount.
#[rule]
fn borrow_increases_debt(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);

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

    cvlr_assert!(pos_after > pos_before);
}

/// Over-repay clears the borrow position.
#[rule]
fn full_repay_clears_debt(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    let pos_before = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &asset,
    );
    cvlr_assume!(pos_before > 0);
    cvlr_assume!(amount > pos_before && amount <= crate::constants::WAD);

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset.clone(), amount);

    let pos_after = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &asset,
    );

    cvlr_assert!(pos_after == 0);
}

/// Withdraw decreases the deposit scaled amount.
#[rule]
fn withdraw_decreases_position(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);

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

    cvlr_assert!(pos_after < pos_before);
}

/// Repay decreases the debt scaled amount.
#[rule]
fn repay_decreases_debt(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);

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

    cvlr_assert!(pos_after < pos_before);
}

#[rule]
fn supply_sanity(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    crate::spec::compat::supply_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(true);
}