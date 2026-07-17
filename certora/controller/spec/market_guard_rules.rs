//! Market entry guards reject new exposure when preconditions fail.

use cvlr::macros::rule;
use cvlr::{cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::WAD;
use crate::types::{AccountPositionType, HubAssetKey};

#[rule]
fn no_collateral_account_cannot_borrow(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let supply_count =
        crate::storage::positions::count_positions(&e, account_id, AccountPositionType::Deposit);
    cvlr_assume!(supply_count == 0);

    crate::spec::compat::borrow_single(e, caller, account_id, asset, amount);

    cvlr_satisfy!(false);
}

#[rule]
fn disabled_market_blocks_new_supply(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };
    cvlr_assume!(crate::storage::get_spoke_asset(&e, 0, &hub_asset).is_none());

    crate::spec::compat::supply_single(e, caller, account_id, asset, amount);

    cvlr_satisfy!(false);
}

#[rule]
fn pending_oracle_market_blocks_new_borrow(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };
    cvlr_assume!(crate::storage::get_spoke_asset(&e, 0, &hub_asset).is_some());
    cvlr_assume!(crate::storage::get_asset_oracle(&e, &asset).is_none());

    crate::spec::compat::borrow_single(e, caller, account_id, asset, amount);

    cvlr_satisfy!(false);
}

#[rule]
fn market_guard_reachability(e: Env, caller: Address, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);
    crate::spec::compat::supply_single(e, caller, 1, asset, amount);
    cvlr_satisfy!(true);
}
