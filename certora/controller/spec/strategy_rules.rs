use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::types::{AccountPositionType, HubAssetKey, StrategySwap};
use controller_interface::ControllerInterface;

fn hub0(asset: Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset,
    }
}

fn nonempty_strategy_swap() -> StrategySwap {
    cvlr_soroban::nondet_bytes1()
}

#[rule]
fn multiply_rejects_same_tokens(
    e: Env,
    caller: Address,
    token: Address,
    debt_to_flash_loan: i128,
    mode: u32,
) {
    let steps = nonempty_strategy_swap();
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!((1..=3).contains(&mode));
    crate::spec::fixture::seed_market(&e, &token);

    crate::spec::compat::multiply_minimal(
        e.clone(),
        caller,
        crate::spec::fixture::SPOKE_ID,
        token.clone(),
        debt_to_flash_loan,
        token.clone(),
        mode,
        steps,
    );

    cvlr_assert!(false);
}

#[rule]
fn multiply_requires_collateralizable(
    e: Env,
    caller: Address,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
) {
    let steps = nonempty_strategy_swap();
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!(collateral_token != debt_token);
    cvlr_assume!((1..=3).contains(&mode));

    crate::spec::fixture::seed_market(&e, &collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);
    let mut stored = crate::storage::get_spoke_asset(
        &e,
        crate::spec::fixture::SPOKE_ID,
        &hub0(collateral_token.clone()),
    )
    .unwrap();
    stored.is_collateralizable = false;
    crate::storage::set_spoke_asset(
        &e,
        crate::spec::fixture::SPOKE_ID,
        &hub0(collateral_token.clone()),
        &stored,
    );

    let mut cache = crate::context::Cache::new(&e);
    let config: common::types::AssetConfig = cache.require_spoke_asset(
        crate::spec::fixture::SPOKE_ID,
        &hub0(collateral_token.clone()),
    );
    cvlr_assume!(!config.is_collateralizable);

    crate::spec::compat::multiply_minimal(
        e.clone(),
        caller,
        crate::spec::fixture::SPOKE_ID,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        mode,
        steps,
    );

    cvlr_assert!(false);
}

#[rule]
fn swap_debt_preserves_directional_bounds(
    e: Env,
    caller: Address,
    account_id: u64,
    existing_debt_token: Address,
    new_debt_amount: i128,
    new_debt_token: Address,
    old_scaled_before: i128,
) {
    let steps = nonempty_strategy_swap();
    cvlr_assume!(new_debt_amount > 0);
    cvlr_assume!(existing_debt_token != new_debt_token);
    cvlr_assume!(old_scaled_before > 0 && old_scaled_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &existing_debt_token);
    crate::spec::fixture::seed_market(&e, &new_debt_token);
    crate::spec::fixture::seed_debt_position(
        &e,
        account_id,
        &existing_debt_token,
        old_scaled_before,
    );
    let new_scaled_before =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &new_debt_token)
            .map(|position| position.scaled_amount)
            .unwrap_or(0);

    crate::Controller::swap_debt(
        e.clone(),
        caller,
        account_id,
        hub0(existing_debt_token.clone()),
        new_debt_amount,
        hub0(new_debt_token.clone()),
        steps,
    );

    let new_pos_after =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &new_debt_token);
    match new_pos_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount >= new_scaled_before),
        None => cvlr_assert!(new_scaled_before == 0),
    }

    let old_pos_after = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &existing_debt_token,
    );
    match old_pos_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount <= old_scaled_before),
        None => cvlr_assert!(true),
    }
}

#[rule]
fn swap_debt_rejects_same_token(
    e: Env,
    caller: Address,
    account_id: u64,
    token: Address,
    new_debt_amount: i128,
) {
    let steps = nonempty_strategy_swap();
    cvlr_assume!(new_debt_amount > 0);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &token);

    crate::Controller::swap_debt(
        e.clone(),
        caller,
        account_id,
        hub0(token.clone()),
        new_debt_amount,
        hub0(token.clone()),
        steps,
    );

    cvlr_assert!(false);
}

#[rule]
fn swap_collateral_preserves_directional_bounds(
    e: Env,
    caller: Address,
    account_id: u64,
    current_collateral: Address,
    from_amount: i128,
    new_collateral: Address,
    old_scaled_before: i128,
) {
    let steps = nonempty_strategy_swap();
    cvlr_assume!(from_amount > 0);
    cvlr_assume!(current_collateral != new_collateral);
    cvlr_assume!(old_scaled_before > 0 && old_scaled_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &current_collateral);
    crate::spec::fixture::seed_market(&e, &new_collateral);
    crate::spec::fixture::seed_supply_position(
        &e,
        account_id,
        &current_collateral,
        old_scaled_before,
    );
    let new_scaled_before = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &new_collateral,
    )
    .map(|position| position.scaled_amount)
    .unwrap_or(0);

    crate::Controller::swap_collateral(
        e.clone(),
        caller,
        account_id,
        hub0(current_collateral.clone()),
        from_amount,
        hub0(new_collateral.clone()),
        steps,
    );

    let new_pos_after = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &new_collateral,
    );
    match new_pos_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount >= new_scaled_before),
        None => cvlr_assert!(new_scaled_before == 0),
    }

    let old_pos_after = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &current_collateral,
    );
    match old_pos_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount <= old_scaled_before),
        None => cvlr_assert!(true),
    }
}

#[rule]
fn swap_collateral_rejects_same_token(
    e: Env,
    caller: Address,
    account_id: u64,
    token: Address,
    from_amount: i128,
) {
    let steps = nonempty_strategy_swap();
    cvlr_assume!(from_amount > 0);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &token);

    crate::Controller::swap_collateral(
        e.clone(),
        caller,
        account_id,
        hub0(token.clone()),
        from_amount,
        hub0(token.clone()),
        steps,
    );

    cvlr_assert!(false);
}

#[rule]
fn repay_with_collateral_never_increases_positions(
    e: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    collateral_scaled_before: i128,
    debt_scaled_before: i128,
) {
    let steps = nonempty_strategy_swap();
    cvlr_assume!(collateral_amount > 0);
    cvlr_assume!(collateral_token != debt_token);
    cvlr_assume!(
        collateral_scaled_before > 0 && collateral_scaled_before <= 20 * common::constants::RAY
    );
    cvlr_assume!(debt_scaled_before > 0 && debt_scaled_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);
    crate::spec::fixture::seed_supply_position(
        &e,
        account_id,
        &collateral_token,
        collateral_scaled_before,
    );
    crate::spec::fixture::seed_debt_position(&e, account_id, &debt_token, debt_scaled_before);

    crate::spec::compat::repay_debt_with_collateral_minimal(
        e.clone(),
        caller,
        account_id,
        collateral_token.clone(),
        collateral_amount,
        debt_token.clone(),
        steps,
    );

    let collateral_after = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &collateral_token,
    );
    match collateral_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount <= collateral_scaled_before),
        None => cvlr_assert!(true),
    }

    let debt_after =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_token);
    match debt_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount <= debt_scaled_before),
        None => cvlr_assert!(true),
    }
}

#[rule]
fn repay_with_collateral_full_close_clears_debt(
    e: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    collateral_scaled_before: i128,
    debt_scaled_before: i128,
) {
    let steps = nonempty_strategy_swap();
    cvlr_assume!(collateral_amount > 0);
    cvlr_assume!(collateral_token != debt_token);
    cvlr_assume!(
        collateral_scaled_before > 0 && collateral_scaled_before <= 20 * common::constants::RAY
    );
    cvlr_assume!(debt_scaled_before > 0 && debt_scaled_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);
    crate::spec::fixture::seed_supply_position(
        &e,
        account_id,
        &collateral_token,
        collateral_scaled_before,
    );
    crate::spec::fixture::seed_debt_position(&e, account_id, &debt_token, debt_scaled_before);

    crate::spec::compat::repay_debt_with_collateral_close(
        e.clone(),
        caller,
        account_id,
        collateral_token,
        collateral_amount,
        debt_token.clone(),
        steps,
    );

    let debt_after =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_token);
    cvlr_assert!(debt_after.is_none());

    let account = crate::storage::get_account(&e, account_id);
    cvlr_assert!(account.borrow_positions.is_empty());
}

#[rule]
fn repay_with_collateral_sanity(
    e: Env,
    caller: Address,
    collateral_token: Address,
    debt_token: Address,
) {
    let steps = nonempty_strategy_swap();
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let collateral_amount = crate::constants::WAD;
    cvlr_assume!(collateral_token != debt_token);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);

    crate::spec::compat::repay_debt_with_collateral_minimal(
        e,
        caller,
        account_id,
        collateral_token,
        collateral_amount,
        debt_token,
        steps,
    );

    cvlr_satisfy!(true);
}

#[rule]
fn clean_bad_debt_zeros_positions(e: Env, account_id: u64) {
    let owner = cvlr_soroban::nondet_address();
    let debt_asset = cvlr_soroban::nondet_address();
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &owner);
    crate::spec::fixture::seed_debt_position(&e, account_id, &debt_asset, 1);

    crate::positions::liquidation::clean_bad_debt_standalone(&e, account_id);

    let deposit_list =
        crate::storage::get_position_list(&e, account_id, AccountPositionType::Deposit);
    let borrow_list =
        crate::storage::get_position_list(&e, account_id, AccountPositionType::Borrow);

    cvlr_assert!(deposit_list.is_empty());
    cvlr_assert!(borrow_list.is_empty());
}

#[rule]
fn claim_revenue_returns_nonnegative_amount(e: Env, caller: Address, asset: Address) {
    crate::spec::fixture::seed_market(&e, &asset);
    let amounts =
        crate::Controller::claim_revenue(e.clone(), caller, soroban_sdk::vec![&e, hub0(asset)]);
    let amount = amounts.get(0).unwrap();

    cvlr_assert!(amount >= 0);
}

#[rule]
fn claim_revenue_sanity(e: Env, caller: Address, asset: Address) {
    crate::spec::fixture::seed_market(&e, &asset);
    let amounts =
        crate::Controller::claim_revenue(e.clone(), caller, soroban_sdk::vec![&e, hub0(asset)]);
    let _amount = amounts.get(0).unwrap();

    cvlr_satisfy!(true);
}

#[rule]
fn multiply_sanity(e: Env, caller: Address, collateral_token: Address, debt_token: Address) {
    let steps = nonempty_strategy_swap();
    let debt_to_flash_loan = crate::constants::WAD;
    cvlr_assume!(collateral_token != debt_token);
    crate::spec::fixture::seed_market(&e, &collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);

    let account_id = crate::spec::compat::multiply_minimal(
        e,
        caller,
        crate::spec::fixture::SPOKE_ID,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        1,
        steps,
    );
    let _account_id = account_id;
    cvlr_satisfy!(true);
}

#[rule]
fn swap_debt_sanity(
    e: Env,
    caller: Address,
    existing_debt_token: Address,
    new_debt_token: Address,
) {
    let steps = nonempty_strategy_swap();
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let new_debt_amount = crate::constants::WAD;
    cvlr_assume!(existing_debt_token != new_debt_token);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &existing_debt_token);
    crate::spec::fixture::seed_market(&e, &new_debt_token);

    crate::Controller::swap_debt(
        e,
        caller,
        account_id,
        hub0(existing_debt_token),
        new_debt_amount,
        hub0(new_debt_token),
        steps,
    );
    cvlr_satisfy!(true);
}

#[rule]
fn swap_collateral_sanity(
    e: Env,
    caller: Address,
    current_collateral: Address,
    new_collateral: Address,
) {
    let steps = nonempty_strategy_swap();
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let from_amount = crate::constants::WAD;
    cvlr_assume!(current_collateral != new_collateral);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &current_collateral);
    crate::spec::fixture::seed_market(&e, &new_collateral);

    crate::Controller::swap_collateral(
        e,
        caller,
        account_id,
        hub0(current_collateral),
        from_amount,
        hub0(new_collateral),
        steps,
    );
    cvlr_satisfy!(true);
}

#[rule]
fn clean_bad_debt_sanity(e: Env) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let owner = cvlr_soroban::nondet_address();
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &owner);
    crate::positions::liquidation::clean_bad_debt_standalone(&e, account_id);
    cvlr_satisfy!(true);
}

#[rule]
fn net_settle_pivot_never_leaves_zero_scaled_records(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    repay_amount: i128,
    collateral_amount: i128,
) {
    cvlr_assume!(repay_amount > 0 && repay_amount <= crate::constants::WAD * 1000);
    cvlr_assume!(collateral_amount > 0 && collateral_amount <= crate::constants::WAD * 1000);
    let steps = nonempty_strategy_swap();
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_supply_position(&e, account_id, &asset, 20 * common::constants::RAY);
    crate::spec::fixture::seed_debt_position(&e, account_id, &asset, 10 * common::constants::RAY);

    crate::spec::compat::repay_debt_with_collateral_minimal(
        e.clone(),
        caller,
        account_id,
        asset.clone(),
        collateral_amount,
        asset.clone(),
        steps,
    );

    // Same-asset repay flows through net_settle_collateral_against_debt
    // (strategies/legs.rs). The pool outcome is merged verbatim, and
    // production removes a record when the merged scaled value is zero
    // (account.rs update_or_remove_*_position), so any surviving record is
    // strictly positive — no orphan zero-scaled debt or supply can persist
    // after the pivot.
    let supply = crate::storage::get_supply_positions(&e, account_id)
        .get(crate::spec::fixture::hub_asset(&asset));
    let debt = crate::storage::get_debt_positions(&e, account_id)
        .get(crate::spec::fixture::hub_asset(&asset));
    cvlr_assert!(supply.map_or(true, |p| p.scaled_amount > 0));
    cvlr_assert!(debt.map_or(true, |p| p.scaled_amount > 0));
}

#[rule]
fn flash_position_rejects_empty_collaterals(
    e: Env,
    caller: Address,
    receiver: Address,
    debt_token: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);
    crate::spec::fixture::seed_market(&e, &debt_token);

    crate::Controller::flash_position(
        e.clone(),
        caller,
        0,
        crate::spec::fixture::SPOKE_ID,
        crate::types::PositionMode::Multiply,
        hub0(debt_token),
        amount,
        receiver,
        soroban_sdk::Bytes::new(&e),
        soroban_sdk::Vec::new(&e),
        soroban_sdk::Vec::new(&e),
    );

    cvlr_assert!(false);
}

#[rule]
fn flash_position_rejects_zero_amount(
    e: Env,
    caller: Address,
    receiver: Address,
    debt_token: Address,
    collateral_token: Address,
) {
    crate::spec::fixture::seed_market(&e, &debt_token);
    crate::spec::fixture::seed_market(&e, &collateral_token);

    crate::spec::compat::flash_position_minimal(
        e,
        caller,
        0,
        crate::spec::fixture::SPOKE_ID,
        1,
        debt_token,
        0,
        receiver,
        collateral_token,
        crate::constants::WAD,
    );

    cvlr_assert!(false);
}

#[rule]
fn flash_position_rejects_all_zero_mins(
    e: Env,
    caller: Address,
    receiver: Address,
    debt_token: Address,
    collateral_token: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);
    crate::spec::fixture::seed_market(&e, &debt_token);
    crate::spec::fixture::seed_market(&e, &collateral_token);

    crate::spec::compat::flash_position_minimal(
        e,
        caller,
        0,
        crate::spec::fixture::SPOKE_ID,
        1,
        debt_token,
        amount,
        receiver,
        collateral_token,
        0,
    );

    cvlr_assert!(false);
}

#[rule]
fn flash_position_rejects_duplicate_collateral_asset(
    e: Env,
    caller: Address,
    receiver: Address,
    debt_token: Address,
    collateral_token: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);
    crate::spec::fixture::seed_market(&e, &debt_token);
    crate::spec::fixture::seed_market(&e, &collateral_token);

    let mut collaterals = soroban_sdk::Vec::new(&e);
    let key = hub0(collateral_token);
    collaterals.push_back((key.clone(), crate::constants::WAD));
    collaterals.push_back((key, crate::constants::WAD));

    crate::Controller::flash_position(
        e.clone(),
        caller,
        0,
        crate::spec::fixture::SPOKE_ID,
        crate::types::PositionMode::Multiply,
        hub0(debt_token),
        amount,
        receiver,
        soroban_sdk::Bytes::new(&e),
        collaterals,
        soroban_sdk::Vec::new(&e),
    );

    cvlr_assert!(false);
}

#[rule]
fn flash_position_guard_blocks_entrypoint(
    e: Env,
    caller: Address,
    receiver: Address,
    debt_token: Address,
    collateral_token: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);
    crate::storage::set_flash_loan_ongoing(&e, true);
    crate::spec::fixture::seed_market(&e, &debt_token);
    crate::spec::fixture::seed_market(&e, &collateral_token);

    crate::spec::compat::flash_position_minimal(
        e,
        caller,
        0,
        crate::spec::fixture::SPOKE_ID,
        1,
        debt_token,
        amount,
        receiver,
        collateral_token,
        crate::constants::WAD,
    );

    cvlr_assert!(false);
}

#[rule]
fn flash_position_open_rejects_empty_account(e: Env, debt_token: Address) {
    let account = crate::types::Account {
        owner: cvlr_soroban::nondet_address(),
        spoke_id: crate::spec::fixture::SPOKE_ID,
        mode: crate::types::PositionMode::Multiply,
        supply_positions: soroban_sdk::Map::new(&e),
        borrow_positions: soroban_sdk::Map::new(&e),
    };
    crate::strategies::flash_position::require_flash_position_still_open(
        &e,
        &account,
        &hub0(debt_token),
    );
    cvlr_assert!(false);
}

#[rule]
fn flash_position_open_rejects_debt_free(e: Env, debt_token: Address, collateral_token: Address) {
    crate::spec::fixture::seed_market(&e, &collateral_token);
    let mut supply = soroban_sdk::Map::new(&e);
    supply.set(
        hub0(collateral_token),
        crate::types::AccountPositionRaw {
            scaled_amount: crate::constants::RAY,
            liquidation_threshold: 8_000,
            liquidation_bonus: 500,
            loan_to_value: 7_500,
            liquidation_fees: 100,
        },
    );
    let account = crate::types::Account {
        owner: cvlr_soroban::nondet_address(),
        spoke_id: crate::spec::fixture::SPOKE_ID,
        mode: crate::types::PositionMode::Multiply,
        supply_positions: supply,
        borrow_positions: soroban_sdk::Map::new(&e),
    };
    crate::strategies::flash_position::require_flash_position_still_open(
        &e,
        &account,
        &hub0(debt_token),
    );
    cvlr_assert!(false);
}

#[rule]
fn flash_position_success_leaves_debt_and_supply(
    e: Env,
    caller: Address,
    receiver: Address,
    debt_token: Address,
    collateral_token: Address,
) {
    let amount = crate::constants::WAD;
    cvlr_assume!(debt_token != collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);
    crate::spec::fixture::seed_market(&e, &collateral_token);

    let account_id = crate::spec::compat::flash_position_minimal(
        e.clone(),
        caller,
        0,
        crate::spec::fixture::SPOKE_ID,
        1,
        debt_token.clone(),
        amount,
        receiver,
        collateral_token.clone(),
        amount,
    );

    let account = crate::storage::get_account(&e, account_id);
    cvlr_assert!(!account.debt_free());
    cvlr_assert!(!account.supply_positions.is_empty());
    let debt = account.borrow_positions.get(hub0(debt_token));
    cvlr_assert!(debt.is_some());
    cvlr_assert!(debt.unwrap().scaled_amount > 0);
}

#[rule]
fn flash_position_does_not_change_other_account(
    e: Env,
    caller: Address,
    receiver: Address,
    debt_token: Address,
    collateral_token: Address,
) {
    let amount = crate::constants::WAD;
    let other_account: u64 = 2;
    cvlr_assume!(debt_token != collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);
    crate::spec::fixture::seed_market(&e, &collateral_token);
    crate::spec::fixture::seed_account(&e, other_account, &caller);

    let other_supply_before = crate::storage::get_supply_positions(&e, other_account).len();
    let other_debt_before = crate::storage::get_debt_positions(&e, other_account).len();

    let _ = crate::spec::compat::flash_position_minimal(
        e.clone(),
        caller,
        0,
        crate::spec::fixture::SPOKE_ID,
        1,
        debt_token,
        amount,
        receiver,
        collateral_token,
        amount,
    );

    cvlr_assert!(
        crate::storage::get_supply_positions(&e, other_account).len() == other_supply_before
    );
    cvlr_assert!(crate::storage::get_debt_positions(&e, other_account).len() == other_debt_before);
}

#[rule]
fn flash_position_sanity(
    e: Env,
    caller: Address,
    receiver: Address,
    debt_token: Address,
    collateral_token: Address,
) {
    cvlr_assume!(debt_token != collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);
    crate::spec::fixture::seed_market(&e, &collateral_token);

    let account_id = crate::spec::compat::flash_position_minimal(
        e,
        caller,
        0,
        crate::spec::fixture::SPOKE_ID,
        1,
        debt_token,
        crate::constants::WAD,
        receiver,
        collateral_token,
        crate::constants::WAD,
    );
    let _ = account_id;
    cvlr_satisfy!(true);
}
