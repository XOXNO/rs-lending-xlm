/// Aave-style controller parity rules.
///
/// These rules cover proof families that are explicit in mature lending
/// protocol Certora suites: user isolation, no-collateral-no-debt, status
/// safety, and controller/pool mutation consistency. They complement the
/// domain-specific controller rules without replacing them.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::WAD;
use common::types::{MarketStatus, POSITION_TYPE_BORROW, POSITION_TYPE_DEPOSIT};

#[rule]
fn no_collateral_account_cannot_borrow(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let supply_count =
        crate::storage::positions::count_positions(&e, account_id, POSITION_TYPE_DEPOSIT);
    cvlr_assume!(supply_count == 0);

    crate::spec::compat::borrow_single(e, caller, account_id, asset, amount);

    cvlr_satisfy!(false);
}

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
        POSITION_TYPE_DEPOSIT,
        &asset,
    );
    let other_borrow_before = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        POSITION_TYPE_BORROW,
        &asset,
    );

    crate::spec::compat::supply_single(e.clone(), caller, target_account, asset.clone(), amount);

    let other_supply_after = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        POSITION_TYPE_DEPOSIT,
        &asset,
    );
    let other_borrow_after = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        POSITION_TYPE_BORROW,
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
        POSITION_TYPE_DEPOSIT,
        &asset,
    );
    let other_borrow_before = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        POSITION_TYPE_BORROW,
        &asset,
    );

    crate::spec::compat::borrow_single(e.clone(), caller, target_account, asset.clone(), amount);

    let other_supply_after = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        POSITION_TYPE_DEPOSIT,
        &asset,
    );
    let other_borrow_after = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        POSITION_TYPE_BORROW,
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
        POSITION_TYPE_BORROW,
        &asset,
    );

    crate::spec::compat::repay_single(e.clone(), caller, target_account, asset.clone(), amount);

    let other_debt_after = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        POSITION_TYPE_BORROW,
        &asset,
    );

    cvlr_assert!(other_debt_after == other_debt_before);
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
fn controller_supply_persists_pool_returned_position(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let before =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_DEPOSIT, &asset);

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset.clone(), amount);

    let after =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_DEPOSIT, &asset);
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

    let before =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_BORROW, &asset);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset.clone(), amount);

    let after =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_BORROW, &asset);
    cvlr_assert!(after >= before);
}

#[rule]
fn controller_parity_reachability(e: Env, caller: Address, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);
    crate::spec::compat::supply_single(e, caller, 1, asset, amount);
    cvlr_satisfy!(true);
}
