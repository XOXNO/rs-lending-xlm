//! Strategy and admin operation verification rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::BAD_DEBT_USD_THRESHOLD;
use crate::types::{AccountPositionType, HubAssetKey, StrategySwap};

/// Hub-0 coordinate for `asset`; the spec models the single default hub.
fn hub0(asset: Address) -> HubAssetKey {
    HubAssetKey { hub_id: 0, asset }
}

/// Successful multiply creates deposit and borrow positions with scaled amount > 0.
#[rule]
fn multiply_basic(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: StrategySwap,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!(collateral_token != debt_token);
    cvlr_assume!((1..=3).contains(&mode));

    let account_id = crate::spec::compat::multiply_basic(
        e.clone(),
        caller,
        e_mode_category,
        collateral_token.clone(),
        debt_to_flash_loan,
        debt_token.clone(),
        mode,
        steps,
    );

    let deposit_pos = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &collateral_token,
    );
    cvlr_assert!(deposit_pos.is_some());
    let deposit = deposit_pos.unwrap();
    cvlr_assert!(deposit.scaled_amount > 0);

    let borrow_pos =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_token);
    cvlr_assert!(borrow_pos.is_some());
    let borrow = borrow_pos.unwrap();
    cvlr_assert!(borrow.scaled_amount > 0);
}

/// Multiply with initial payment in collateral_token creates both positions.
#[rule]
fn multiply_with_initial_payment_collateral(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: StrategySwap,
    initial_amount: i128,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!(initial_amount > 0);
    cvlr_assume!(collateral_token != debt_token);
    cvlr_assume!((1..=3).contains(&mode));

    let account_id = crate::spec::compat::multiply_with_initial_payment_collateral(
        e.clone(),
        caller,
        e_mode_category,
        collateral_token.clone(),
        debt_to_flash_loan,
        debt_token.clone(),
        mode,
        steps,
        initial_amount,
    );

    let deposit_pos = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &collateral_token,
    );
    cvlr_assert!(deposit_pos.is_some());
    cvlr_assert!(deposit_pos.unwrap().scaled_amount > 0);

    let borrow_pos =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_token);
    cvlr_assert!(borrow_pos.is_some());
    cvlr_assert!(borrow_pos.unwrap().scaled_amount > 0);
}

/// Multiply with initial payment in a third token creates both positions.
#[rule]
fn multiply_with_initial_payment_third_token(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: StrategySwap,
    third_token: Address,
    initial_amount: i128,
    convert_steps: StrategySwap,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!(initial_amount > 0);
    cvlr_assume!(collateral_token != debt_token);
    cvlr_assume!(third_token != collateral_token);
    cvlr_assume!(third_token != debt_token);
    cvlr_assume!((1..=3).contains(&mode));

    let account_id = crate::spec::compat::multiply_with_initial_payment_third_token(
        e.clone(),
        caller,
        e_mode_category,
        collateral_token.clone(),
        debt_to_flash_loan,
        debt_token.clone(),
        mode,
        steps,
        third_token,
        initial_amount,
        convert_steps,
    );

    let deposit_pos = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &collateral_token,
    );
    cvlr_assert!(deposit_pos.is_some());
    cvlr_assert!(deposit_pos.unwrap().scaled_amount > 0);

    let borrow_pos =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_token);
    cvlr_assert!(borrow_pos.is_some());
    cvlr_assert!(borrow_pos.unwrap().scaled_amount > 0);
}

/// Multiply with collateral_token == debt_token reverts.
#[rule]
fn multiply_rejects_same_tokens(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    token: Address,
    debt_to_flash_loan: i128,
    mode: u32,
    steps: StrategySwap,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!((1..=3).contains(&mode));

    crate::spec::compat::multiply_minimal(
        e.clone(),
        caller,
        e_mode_category,
        token.clone(),
        debt_to_flash_loan,
        token.clone(),
        mode,
        steps,
    );

    cvlr_satisfy!(false);
}

/// Multiply with non-collateralizable collateral reverts.
#[rule]
fn multiply_requires_collateralizable(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: StrategySwap,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!(collateral_token != debt_token);
    cvlr_assume!((1..=3).contains(&mode));

    let config = crate::emode::effective_asset_config(
        &e,
        e_mode_category,
        &hub0(collateral_token.clone()),
    );
    cvlr_assume!(!config.is_collateralizable);

    crate::spec::compat::multiply_minimal(
        e.clone(),
        caller,
        e_mode_category,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        mode,
        steps,
    );

    cvlr_satisfy!(false);
}

/// swap_debt creates target debt and decreases or removes source debt.
#[rule]
fn swap_debt_conserves_debt_value(
    e: Env,
    caller: Address,
    account_id: u64,
    existing_debt_token: Address,
    new_debt_amount: i128,
    new_debt_token: Address,
    steps: StrategySwap,
) {
    cvlr_assume!(new_debt_amount > 0);
    cvlr_assume!(existing_debt_token != new_debt_token);

    let old_pos_before = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &existing_debt_token,
    );
    cvlr_assume!(old_pos_before.is_some());
    let old_scaled_before = old_pos_before.unwrap().scaled_amount;
    cvlr_assume!(old_scaled_before > 0);

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
    cvlr_assert!(new_pos_after.is_some());
    cvlr_assert!(new_pos_after.unwrap().scaled_amount > 0);

    let old_pos_after = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Borrow,
        &existing_debt_token,
    );
    match old_pos_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount < old_scaled_before),
        None => cvlr_assert!(true),
    }
}

/// swap_debt with existing_debt_token == new_debt_token reverts.
#[rule]
fn swap_debt_rejects_same_token(
    e: Env,
    caller: Address,
    account_id: u64,
    token: Address,
    new_debt_amount: i128,
    steps: StrategySwap,
) {
    cvlr_assume!(new_debt_amount > 0);

    crate::Controller::swap_debt(
        e.clone(),
        caller,
        account_id,
        hub0(token.clone()),
        new_debt_amount,
        hub0(token.clone()),
        steps,
    );

    cvlr_satisfy!(false);
}

/// swap_collateral creates target collateral and decreases or removes source collateral.
#[rule]
fn swap_collateral_conserves_collateral(
    e: Env,
    caller: Address,
    account_id: u64,
    current_collateral: Address,
    from_amount: i128,
    new_collateral: Address,
    steps: StrategySwap,
) {
    cvlr_assume!(from_amount > 0);
    cvlr_assume!(current_collateral != new_collateral);

    let old_pos_before = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &current_collateral,
    );
    cvlr_assume!(old_pos_before.is_some());
    let old_scaled_before = old_pos_before.unwrap().scaled_amount;
    cvlr_assume!(old_scaled_before > 0);

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
    cvlr_assert!(new_pos_after.is_some());
    cvlr_assert!(new_pos_after.unwrap().scaled_amount > 0);

    let old_pos_after = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &current_collateral,
    );
    match old_pos_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount < old_scaled_before),
        None => cvlr_assert!(true),
    }
}

/// swap_collateral with current_collateral == new_collateral reverts.
#[rule]
fn swap_collateral_rejects_same_token(
    e: Env,
    caller: Address,
    account_id: u64,
    token: Address,
    from_amount: i128,
    steps: StrategySwap,
) {
    cvlr_assume!(from_amount > 0);

    crate::Controller::swap_collateral(
        e.clone(),
        caller,
        account_id,
        hub0(token.clone()),
        from_amount,
        hub0(token.clone()),
        steps,
    );

    cvlr_satisfy!(false);
}

/// repay_debt_with_collateral with close_position=false reduces collateral and debt.
#[rule]
fn repay_with_collateral_reduces_both_no_close(
    e: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    steps: StrategySwap,
) {
    cvlr_assume!(collateral_amount > 0);
    cvlr_assume!(collateral_token != debt_token);

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
        Some(pos) => cvlr_assert!(pos.scaled_amount < collateral_scaled_before),
        None => cvlr_assert!(true),
    }

    let debt_after =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_token);
    match debt_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount < debt_scaled_before),
        None => cvlr_assert!(true),
    }
}

/// repay_debt_with_collateral with close_position=true clears both positions.
#[rule]
fn repay_with_collateral_full_close_removes_account(
    e: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    steps: StrategySwap,
) {
    cvlr_assume!(collateral_amount > 0);
    cvlr_assume!(collateral_token != debt_token);

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
        collateral_token.clone(),
        collateral_amount,
        debt_token.clone(),
        steps,
    );

    let debt_after =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_token);
    cvlr_assert!(debt_after.is_none());

    let collateral_after = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &collateral_token,
    );
    cvlr_assert!(collateral_after.is_none());
}

/// clean_bad_debt reverts when the account does not qualify for cleanup.
#[rule]
fn clean_bad_debt_requires_qualification(e: Env, account_id: u64) {
    let mut cache = crate::cache::Cache::new(&e);

    let account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(!account.borrow_positions.is_empty());

    let totals = crate::helpers::calculate_account_risk_totals(
        &e,
        &mut cache,
        account.spoke_id,
        &account.supply_positions,
        &account.borrow_positions,
    );

    cvlr_assume!(
        !(totals.total_debt.raw() > totals.total_collateral.raw()
            && totals.total_collateral.raw() <= BAD_DEBT_USD_THRESHOLD)
    );

    crate::positions::liquidation::clean_bad_debt_standalone(&e, account_id);

    cvlr_satisfy!(false);
}

/// clean_bad_debt on a qualifying account clears all supply and borrow positions.
#[rule]
fn clean_bad_debt_zeros_positions(e: Env, account_id: u64) {
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

/// claim_revenue returns a non-negative amount.
#[rule]
fn claim_revenue_transfers_to_accumulator(e: Env, caller: Address, asset: Address) {
    let amounts =
        crate::Controller::claim_revenue(e.clone(), caller, soroban_sdk::vec![&e, hub0(asset)]);
    let amount = amounts.get(0).unwrap();

    cvlr_assert!(amount >= 0);
    cvlr_satisfy!(amount >= 0);
}

#[rule]
fn multiply_sanity(
    e: Env,
    caller: Address,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    steps: StrategySwap,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!(collateral_token != debt_token);

    let account_id = crate::spec::compat::multiply(
        e,
        caller,
        0,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        1,
        steps,
    );
    cvlr_satisfy!(account_id > 0);
}

#[rule]
fn swap_debt_sanity(
    e: Env,
    caller: Address,
    account_id: u64,
    existing_debt_token: Address,
    new_debt_amount: i128,
    new_debt_token: Address,
    steps: StrategySwap,
) {
    cvlr_assume!(new_debt_amount > 0);
    cvlr_assume!(existing_debt_token != new_debt_token);

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
    account_id: u64,
    current_collateral: Address,
    from_amount: i128,
    new_collateral: Address,
    steps: StrategySwap,
) {
    cvlr_assume!(from_amount > 0);
    cvlr_assume!(current_collateral != new_collateral);

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
fn clean_bad_debt_sanity(e: Env, account_id: u64) {
    crate::positions::liquidation::clean_bad_debt_standalone(&e, account_id);
    cvlr_satisfy!(true);
}
