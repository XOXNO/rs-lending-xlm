/// Market entry-guard rules.
///
/// Prove that the controller rejects new exposure when a precondition fails:
/// an account with no collateral cannot borrow, a disabled market accepts no
/// new supply, and a market with a pending oracle accepts no new borrow. Each
/// rule asserts the guarded path is unreachable (`cvlr_satisfy!(false)`).
use cvlr::macros::rule;
use cvlr::{cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::WAD;
use crate::types::{AccountPositionType, MarketStatus};

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

    let market = crate::storage::get_market_config(&e, &asset);
    cvlr_assume!(market.status == MarketStatus::Disabled);

    crate::spec::compat::supply_single(e, caller, account_id, asset, amount);

    cvlr_satisfy!(false);
}

#[rule]
fn pending_oracle_market_blocks_new_borrow(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let market = crate::storage::get_market_config(&e, &asset);
    cvlr_assume!(market.status == MarketStatus::PendingOracle);

    crate::spec::compat::borrow_single(e, caller, account_id, asset, amount);

    cvlr_satisfy!(false);
}

#[rule]
fn market_guard_reachability(e: Env, caller: Address, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);
    crate::spec::compat::supply_single(e, caller, 1, asset, amount);
    cvlr_satisfy!(true);
}
