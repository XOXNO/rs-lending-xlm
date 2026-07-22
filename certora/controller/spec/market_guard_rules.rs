//! Market entry guards reject new exposure when preconditions fail.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::WAD;
use crate::types::{AccountPositionType, HubAssetKey};

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

/// Third parties may top up existing supply legs but must not open a new
/// asset slot on someone else's account (slot-griefing guard in
/// `process_supply`). Caller is neither owner nor a registered delegate and
/// the account has no position in `asset`, so supply must revert.
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

#[rule]
fn market_guard_reachability(e: Env, caller: Address, asset: Address) {
    let amount = WAD;
    crate::spec::fixture::seed_live_account(&e, 1, &caller, &asset);
    crate::spec::compat::supply_single(e, caller, 1, asset, amount);
    cvlr_satisfy!(true);
}
