//! `MockAuth` argument builders for nested C2C auth (e.g. receiver `token.mint()`)
//! that `env.mock_all_auths()` cannot reach in recording mode.
//!
//! Returns owned arg `Vec`s; callers own them, assemble `MockAuthInvoke` refs,
//! and pass `&[MockAuth]` to `mock_auths`.

use soroban_sdk::{Address, Env, IntoVal, Val, Vec};

/// Args for `Controller::flash_loan(caller, asset, amount, receiver, data)`.
pub fn flash_loan_args(
    env: &Env,
    caller: &Address,
    asset: &Address,
    amount: i128,
    receiver: &Address,
) -> Vec<Val> {
    (
        caller.clone(),
        asset.clone(),
        amount,
        receiver.clone(),
        soroban_sdk::Bytes::new(env),
    )
        .into_val(env)
}

/// Args for `Controller::multiply` (initial_payment/convert_steps = None).
#[allow(clippy::too_many_arguments)]
pub fn multiply_args(
    env: &Env,
    caller: &Address,
    account_id: u64,
    spoke_id: u32,
    collateral_token: &Address,
    debt_amount: i128,
    debt_token: &Address,
    mode: controller::types::PositionMode,
    steps: &controller::types::StrategySwap,
) -> Vec<Val> {
    (
        caller.clone(),
        account_id,
        spoke_id,
        collateral_token.clone(),
        debt_amount,
        debt_token.clone(),
        mode,
        steps.clone(),
        None::<(Address, i128)>,
        None::<controller::types::StrategySwap>,
    )
        .into_val(env)
}

/// Arguments for `Controller::swap_collateral(caller, account_id,
/// current_collateral, from_amount, new_collateral, steps)`.
pub fn swap_collateral_args(
    env: &Env,
    caller: &Address,
    account_id: u64,
    current_collateral: &Address,
    from_amount: i128,
    new_collateral: &Address,
    steps: &controller::types::StrategySwap,
) -> Vec<Val> {
    (
        caller.clone(),
        account_id,
        current_collateral.clone(),
        from_amount,
        new_collateral.clone(),
        steps.clone(),
    )
        .into_val(env)
}

/// Arguments for `Controller::swap_debt`.
pub fn swap_debt_args(
    env: &Env,
    caller: &Address,
    account_id: u64,
    existing_debt: &Address,
    new_amount: i128,
    new_debt: &Address,
    steps: &controller::types::StrategySwap,
) -> Vec<Val> {
    (
        caller.clone(),
        account_id,
        existing_debt.clone(),
        new_amount,
        new_debt.clone(),
        steps.clone(),
    )
        .into_val(env)
}

/// Arguments for `Controller::repay_debt_with_collateral`.
#[allow(clippy::too_many_arguments)]
pub fn repay_debt_with_collateral_args(
    env: &Env,
    caller: &Address,
    account_id: u64,
    collateral: &Address,
    collateral_amount: i128,
    debt: &Address,
    steps: &controller::types::StrategySwap,
    close_position: bool,
) -> Vec<Val> {
    (
        caller.clone(),
        account_id,
        collateral.clone(),
        collateral_amount,
        debt.clone(),
        steps.clone(),
        close_position,
    )
        .into_val(env)
}
