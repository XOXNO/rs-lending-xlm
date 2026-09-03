use cvlr::macros::rule;
use cvlr::nondet::nondet;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::spec::fixture;
use crate::types::AccountPositionType;

/// Bounds `account_id`'s books to at most the one position the direction rules
/// read, keeping both production branches reachable: `held == false` is the
/// new-slot path, `held == true` the top-up path. Excludes books that hold a
/// second asset; the frame rules in `account_isolation_rules.rs` keep the
/// unbounded form.
fn seed_single_asset_book(
    e: &Env,
    account_id: u64,
    asset: &Address,
    position: AccountPositionType,
) {
    fixture::seed_empty_books(e, account_id);
    let held: bool = nondet();
    if held {
        let scaled: i128 = nondet();
        cvlr_assume!(scaled > 0 && scaled <= 20 * common::constants::RAY);
        match position {
            AccountPositionType::Deposit => {
                fixture::seed_supply_position(e, account_id, asset, scaled)
            }
            AccountPositionType::Borrow => {
                fixture::seed_debt_position(e, account_id, asset, scaled)
            }
        }
    }
}

#[rule]
fn supply_does_not_decrease_position(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    seed_single_asset_book(&e, account_id, &asset, AccountPositionType::Deposit);

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
    seed_single_asset_book(&e, account_id, &asset, AccountPositionType::Borrow);

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
fn withdraw_does_not_increase_position(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
    pos_before: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(pos_before > 0 && pos_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    fixture::seed_empty_books(&e, account_id);
    crate::spec::fixture::seed_supply_position(&e, account_id, &asset, pos_before);

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
fn repay_does_not_increase_debt(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
    pos_before: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(pos_before > 0 && pos_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    fixture::seed_empty_books(&e, account_id);
    crate::spec::fixture::seed_debt_position(&e, account_id, &asset, pos_before);

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

#[rule]
fn withdraw_after_borrow_preserves_debt_record(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    pos_before: i128,
    borrow_amount: i128,
    withdraw_amount: i128,
) {
    cvlr_assume!(pos_before > 0 && pos_before <= 20 * common::constants::RAY);
    cvlr_assume!(borrow_amount > 0 && borrow_amount <= crate::constants::WAD * 1000);
    cvlr_assume!(withdraw_amount > 0 && withdraw_amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    fixture::seed_empty_books(&e, account_id);
    crate::spec::fixture::seed_supply_position(&e, account_id, &asset, pos_before);

    crate::spec::compat::borrow_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        borrow_amount,
    );

    let hub = crate::spec::fixture::hub_asset(&asset);
    let mid = crate::storage::get_debt_positions(&e, account_id).get(hub.clone());
    cvlr_assume!(mid.is_some());

    crate::spec::compat::withdraw_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        withdraw_amount,
    );

    // Two-step induction: a withdraw acts only on the supply book, so the
    // debt record produced by the preceding borrow must be exactly unchanged
    // on every path where both calls complete.
    let post = crate::storage::get_debt_positions(&e, account_id).get(hub);
    cvlr_assert!(post.is_some());
    cvlr_assert!(post.unwrap().scaled_amount == mid.unwrap().scaled_amount);
}
