use controller_interface::ControllerInterface;
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use cvlr_soroban::nondet_address;
use soroban_sdk::{Address, Env, Vec};

use crate::constants::WAD;
use crate::spec::fixture;
use crate::storage;
use crate::types::{AccountPositionType, HubAssetKey};

const SEEDED_COLLATERAL_RAY: i128 = 10 * common::constants::RAY;

/// A live account whose books hold exactly one collateral position in `asset`.
///
/// The recipient rules need a book on which the verb can otherwise complete,
/// so that the only thing left to reject is the recipient. `seed_empty_books`
/// excludes every other position the havoced book could carry; the frame rules
/// in `account_isolation_rules.rs` keep the unbounded form.
fn seed_collateralized_account(e: &Env, account_id: u64, owner: &Address, asset: &Address) {
    fixture::seed_live_account(e, account_id, owner, asset);
    fixture::seed_empty_books(e, account_id);
    fixture::seed_supply_position(e, account_id, asset, SEEDED_COLLATERAL_RAY);
}

fn one_leg(e: &Env, asset: &Address, amount: i128) -> Vec<(HubAssetKey, i128)> {
    let mut legs = Vec::new(e);
    legs.push_back((fixture::hub_asset(asset), amount));
    legs
}

#[rule]
fn no_collateral_account_cannot_borrow(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let supply_count =
        crate::storage::positions::count_positions(&e, account_id, AccountPositionType::Deposit);
    cvlr_assume!(supply_count == 0);

    crate::spec::compat::borrow_single(e, caller, account_id, asset, amount);

    cvlr_assert!(false);
}

#[rule]
fn disabled_market_blocks_new_supply(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);

    let hub_asset = HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset: asset.clone(),
    };
    cvlr_assume!(
        crate::storage::get_spoke_asset(&e, crate::spec::fixture::SPOKE_ID, &hub_asset).is_none()
    );

    crate::spec::compat::supply_single(e, caller, account_id, asset, amount);

    cvlr_assert!(false);
}

#[rule]
fn supply_new_slot_requires_owner_or_delegate(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let owner = cvlr_soroban::nondet_address();
    cvlr_assume!(caller != owner);
    crate::spec::fixture::seed_live_account(&e, account_id, &owner, &asset);

    let account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(crate::storage::get_position_manager(&e, &caller).is_none());

    let hub_asset = HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset: asset.clone(),
    };
    cvlr_assume!(!account.supply_positions.contains_key(hub_asset));

    crate::spec::compat::supply_single(e, caller, account_id, asset, amount);

    cvlr_assert!(false);
}

/// Satisfy twin of `no_collateral_account_cannot_borrow`: the same account
/// with the gate flipped — one collateral position instead of none — reaches a
/// persisted debt record, so the revert rule is about the empty supply book.
#[rule]
fn no_collateral_account_cannot_borrow_fixture_completes(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    seed_collateralized_account(&e, account_id, &caller, &asset);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset.clone(), WAD);

    cvlr_satisfy!(storage::get_debt_positions(&e, account_id)
        .get(fixture::hub_asset(&asset))
        .is_some());
}

/// A borrow addressed to the pool is refused before any transfer: the pool
/// would debit cash while its token balance does not move.
///
/// Fixture: one collateral position and a positive amount, so the borrow could
/// otherwise complete and the recipient gate is the only reason it cannot.
#[rule]
fn borrow_rejects_pool_recipient(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    seed_collateralized_account(&e, account_id, &caller, &asset);

    let pool = storage::get_pool(&e);
    crate::Controller::borrow(
        e.clone(),
        caller,
        account_id,
        one_leg(&e, &asset, amount),
        Some(pool),
    );

    cvlr_assert!(false);
}

/// Satisfy twin of `borrow_rejects_pool_recipient`: the same fixture with a
/// recipient that is neither protocol contract completes.
#[rule]
fn borrow_rejects_pool_recipient_fixture_completes(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    seed_collateralized_account(&e, account_id, &caller, &asset);

    let recipient = nondet_address();
    cvlr_assume!(recipient != storage::get_pool(&e));
    cvlr_assume!(recipient != e.current_contract_address());

    crate::Controller::borrow(
        e.clone(),
        caller,
        account_id,
        one_leg(&e, &asset, WAD),
        Some(recipient),
    );

    cvlr_satisfy!(storage::get_debt_positions(&e, account_id)
        .get(fixture::hub_asset(&asset))
        .is_some());
}

/// A withdrawal addressed to the controller is refused before any transfer:
/// the tokens would sit in the controller where no balance-delta measurement
/// ever claims them.
#[rule]
fn withdraw_rejects_controller_recipient(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    seed_collateralized_account(&e, account_id, &caller, &asset);

    crate::Controller::withdraw(
        e.clone(),
        caller,
        account_id,
        one_leg(&e, &asset, amount),
        Some(e.current_contract_address()),
    );

    cvlr_assert!(false);
}

/// Satisfy twin of `withdraw_rejects_controller_recipient`: the same fixture
/// paying an external recipient completes.
#[rule]
fn withdraw_rejects_controller_recipient_fixture_completes(
    e: Env,
    caller: Address,
    asset: Address,
) {
    let account_id: u64 = 1;
    seed_collateralized_account(&e, account_id, &caller, &asset);

    let recipient = nondet_address();
    cvlr_assume!(recipient != storage::get_pool(&e));
    cvlr_assume!(recipient != e.current_contract_address());

    crate::Controller::withdraw(
        e.clone(),
        caller,
        account_id,
        one_leg(&e, &asset, WAD),
        Some(recipient),
    );

    cvlr_satisfy!(true);
}

#[rule]
fn market_guard_reachability(e: Env, caller: Address, asset: Address) {
    let amount = WAD;
    crate::spec::fixture::seed_live_account(&e, 1, &caller, &asset);
    crate::spec::compat::supply_single(e, caller, 1, asset, amount);
    cvlr_satisfy!(true);
}
