/// Account isolation rules.
///
/// Prove that a controller action on one account never mutates another
/// account's positions: supply and borrow leave other accounts untouched, and
/// repay only changes the target account's debt. These are cross-account
/// non-interference invariants expected of any multi-account lending market.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::WAD;
use crate::types::AccountPositionType;

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
