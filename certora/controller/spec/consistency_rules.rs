use cvlr::macros::rule;
use cvlr::nondet::nondet;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use crate::constants::WAD;
use crate::spec::fixture;
use crate::types::AccountPositionType;

/// Empties `account_id`'s books, then optionally re-seeds the one asset the
/// rule watches, so both the new-slot and the top-up branch stay reachable on
/// a book of known size. Excludes any second asset the account might hold.
fn seed_watched_asset_only(e: &Env, account_id: u64, asset: &Address, borrow: bool) {
    fixture::seed_empty_books(e, account_id);
    let held: bool = nondet();
    if held {
        let scaled: i128 = nondet();
        cvlr_assume!(scaled > 0 && scaled <= 20 * common::constants::RAY);
        if borrow {
            fixture::seed_debt_position(e, account_id, asset, scaled);
        } else {
            fixture::seed_supply_position(e, account_id, asset, scaled);
        }
    }
}

#[rule]
fn controller_supply_persists_pool_returned_position(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    seed_watched_asset_only(&e, account_id, &asset, false);

    let before = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &asset,
    );

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset.clone(), amount);

    let after = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &asset,
    );
    cvlr_assert!(after >= before);
}

#[rule]
fn controller_borrow_persists_pool_returned_position(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    seed_watched_asset_only(&e, account_id, &asset, true);

    let before = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &asset,
    );

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset.clone(), amount);

    let after = crate::storage::positions::get_scaled_amount(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &asset,
    );
    cvlr_assert!(after >= before);
}
