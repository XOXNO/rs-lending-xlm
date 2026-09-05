//! The `FlashPositionClosed` guard.
//!
//! `require_flash_position_still_open` is the last-line defense against a
//! callback-plus-later-repay round trip leaving an empty account behind: the
//! receiver is handed borrowed funds mid-transaction and could, without this,
//! unwind the position it was opened against before control returns. All three
//! of its arms were dead.
extern crate std;

use super::*;

use common::types::{Account, AccountPositionRaw, DebtPositionRaw, HubAssetKey, PositionMode};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Map};

use crate::Controller;

fn key(env: &Env) -> HubAssetKey {
    HubAssetKey {
        hub_id: 1,
        asset: Address::generate(env),
    }
}

fn account(env: &Env) -> Account {
    Account {
        owner: Address::generate(env),
        spoke_id: 1,
        mode: PositionMode::Multiply,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    }
}

fn supply() -> AccountPositionRaw {
    AccountPositionRaw {
        scaled_amount: 1,
        liquidation_threshold: 8_000,
        liquidation_bonus: 500,
        loan_to_value: 7_500,
        liquidation_fees: 100,
    }
}

fn in_controller<T>(env: &Env, body: impl FnOnce() -> T) -> T {
    let admin = Address::generate(env);
    let id = env.register(Controller, (admin,));
    env.as_contract(&id, body)
}

#[test]
#[should_panic(expected = "Error(Contract, #505)")]
fn a_fully_unwound_account_is_rejected() {
    let env = Env::default();
    let debt = key(&env);
    let acct = account(&env);
    // Nothing left at all: the receiver repaid and withdrew inside its callback.
    in_controller(&env, || {
        require_flash_position_still_open(&env, &acct, &debt);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #505)")]
fn an_account_left_with_collateral_but_no_debt_is_rejected() {
    let env = Env::default();
    let debt = key(&env);
    let mut acct = account(&env);
    acct.supply_positions.set(key(&env), supply());
    // Not empty, but debt-free: the flash borrow was repaid inside the
    // callback, so there is no flash position left to keep open.
    in_controller(&env, || {
        require_flash_position_still_open(&env, &acct, &debt);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #505)")]
fn debt_in_a_different_asset_does_not_keep_this_flash_position_open() {
    let env = Env::default();
    let debt = key(&env);
    let other = key(&env);
    let mut acct = account(&env);
    acct.supply_positions.set(key(&env), supply());
    acct.borrow_positions
        .set(other, DebtPositionRaw { scaled_amount: 1 });
    // The account carries debt, just not in the asset that was flash-borrowed.
    // Accepting this would let a receiver swap the obligation to another asset.
    in_controller(&env, || {
        require_flash_position_still_open(&env, &acct, &debt);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #505)")]
fn a_debt_entry_scaled_to_zero_is_rejected() {
    let env = Env::default();
    let debt = key(&env);
    let mut acct = account(&env);
    acct.supply_positions.set(key(&env), supply());
    // A residual map entry with nothing behind it is not an open position.
    acct.borrow_positions
        .set(debt.clone(), DebtPositionRaw { scaled_amount: 0 });
    in_controller(&env, || {
        require_flash_position_still_open(&env, &acct, &debt);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #505)")]
fn debt_without_any_collateral_is_rejected() {
    let env = Env::default();
    let debt = key(&env);
    let mut acct = account(&env);
    acct.borrow_positions
        .set(debt.clone(), DebtPositionRaw { scaled_amount: 1 });
    // Debt with nothing backing it: the receiver kept the borrow and removed
    // every deposit.
    in_controller(&env, || {
        require_flash_position_still_open(&env, &acct, &debt);
    });
}

#[test]
fn a_live_position_in_the_flashed_asset_passes() {
    let env = Env::default();
    let debt = key(&env);
    let mut acct = account(&env);
    acct.supply_positions.set(key(&env), supply());
    acct.borrow_positions
        .set(debt.clone(), DebtPositionRaw { scaled_amount: 1 });
    // Not panicking is the assertion; the reject probes above pin each arm.
    in_controller(&env, || {
        require_flash_position_still_open(&env, &acct, &debt);
    });
}
