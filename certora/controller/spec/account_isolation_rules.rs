//! Cross-account non-interference: one account's action does not mutate another's positions.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::WAD;
use crate::types::AccountPositionType;

/// Supply on one account leaves other accounts' positions unchanged.
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

    let other_supply_before = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Deposit,
        &asset,
    );
    let other_borrow_before = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Borrow,
        &asset,
    );

    crate::spec::compat::supply_single(e.clone(), caller, target_account, asset.clone(), amount);

    let other_supply_after = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Deposit,
        &asset,
    );
    let other_borrow_after = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Borrow,
        &asset,
    );

    cvlr_assert!(other_supply_after == other_supply_before);
    cvlr_assert!(other_borrow_after == other_borrow_before);
}

/// Borrow on one account leaves other accounts' positions unchanged.
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

    let other_supply_before = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Deposit,
        &asset,
    );
    let other_borrow_before = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Borrow,
        &asset,
    );

    crate::spec::compat::borrow_single(e.clone(), caller, target_account, asset.clone(), amount);

    let other_supply_after = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Deposit,
        &asset,
    );
    let other_borrow_after = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Borrow,
        &asset,
    );

    cvlr_assert!(other_supply_after == other_supply_before);
    cvlr_assert!(other_borrow_after == other_borrow_before);
}

/// Repay on one account does not change another account's debt.
#[rule]
fn repay_only_changes_target_account_debt(e: Env, caller: Address, asset: Address, amount: i128) {
    let target_account: u64 = 1;
    let other_account: u64 = 2;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let other_debt_before = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Borrow,
        &asset,
    );

    crate::spec::compat::repay_single(e.clone(), caller, target_account, asset.clone(), amount);

    let other_debt_after = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Borrow,
        &asset,
    );

    cvlr_assert!(other_debt_after == other_debt_before);
}

#[rule]
fn account_isolation_reachability(e: Env, caller: Address, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);
    crate::spec::compat::supply_single(e, caller, 1, asset, amount);
    cvlr_satisfy!(true);
}
