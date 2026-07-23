//! Strategy and admin operation verification rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::types::{AccountPositionType, HubAssetKey, StrategySwap};

/// Primary-hub coordinate for `asset`.
fn hub0(asset: Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset,
    }
}

/// Strategy routes are opaque to the controller; locally only non-emptiness is
/// observed, so one valid symbolic byte represents the complete non-empty class.
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
    let config: common::types::AssetConfig = (&cache.require_spoke_asset(
        crate::spec::fixture::SPOKE_ID,
        &hub0(collateral_token.clone()),
    ))
        .into();
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
) {
    let steps = nonempty_strategy_swap();
    cvlr_assume!(new_debt_amount > 0);
    cvlr_assume!(existing_debt_token != new_debt_token);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &existing_debt_token);
    crate::spec::fixture::seed_market(&e, &new_debt_token);

    let old_pos_before = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &existing_debt_token,
    );
    cvlr_assume!(old_pos_before.is_some());
    let old_scaled_before = old_pos_before.unwrap().scaled_amount;
    cvlr_assume!(old_scaled_before > 0);
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
) {
    let steps = nonempty_strategy_swap();
    cvlr_assume!(from_amount > 0);
    cvlr_assume!(current_collateral != new_collateral);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &current_collateral);
    crate::spec::fixture::seed_market(&e, &new_collateral);

    let old_pos_before = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &current_collateral,
    );
    cvlr_assume!(old_pos_before.is_some());
    let old_scaled_before = old_pos_before.unwrap().scaled_amount;
    cvlr_assume!(old_scaled_before > 0);
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

/// Repay-with-collateral (no close) never grows either leg: the flow only
/// withdraws collateral and repays debt. Bounds are the summary-contract
/// envelope (`withdraw_summary` / `repay_summary` permit rounding no-ops, so
/// strict decrease is not expressible here); a removed position counts as
/// reduced.
#[rule]
fn repay_with_collateral_never_increases_positions(
    e: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
) {
    let steps = nonempty_strategy_swap();
    cvlr_assume!(collateral_amount > 0);
    cvlr_assume!(collateral_token != debt_token);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);

    let collateral_before = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &collateral_token,
    );
    cvlr_assume!(collateral_before.is_some());
    let collateral_scaled_before = collateral_before.unwrap().scaled_amount;
    cvlr_assume!(collateral_scaled_before > 0);

    let debt_before =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_token);
    cvlr_assume!(debt_before.is_some());
    let debt_scaled_before = debt_before.unwrap().scaled_amount;
    cvlr_assume!(debt_scaled_before > 0);

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

/// Full close clears all debt: `close_position = true` asserts the account's
/// borrow map is empty before withdrawing collateral, so post-state has no
/// debt position for the repaid asset and an empty borrow map.
#[rule]
fn repay_with_collateral_full_close_clears_debt(
    e: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
) {
    let steps = nonempty_strategy_swap();
    cvlr_assume!(collateral_amount > 0);
    cvlr_assume!(collateral_token != debt_token);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);

    let collateral_before = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &collateral_token,
    );
    cvlr_assume!(collateral_before.is_some());
    cvlr_assume!(collateral_before.unwrap().scaled_amount > 0);

    let debt_before =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_token);
    cvlr_assume!(debt_before.is_some());
    cvlr_assume!(debt_before.unwrap().scaled_amount > 0);

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
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &owner);
    let borrow_list_pre =
        crate::storage::get_position_list(&e, account_id, AccountPositionType::Borrow);
    cvlr_assume!(!borrow_list_pre.is_empty());

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
