//! Controller persists pool-returned position updates after supply and borrow.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use crate::constants::WAD;
use crate::types::AccountPositionType;

#[rule]
fn controller_supply_persists_pool_returned_position(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

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
