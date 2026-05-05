//! Explicit `MockAuth` tree builders for fuzz harnesses that need to
//! authorize nested contract-to-contract calls (e.g. the good flash-loan
//! receiver's nested `token.mint()`), which `env.mock_all_auths()` cannot
//! reach in recording mode.
//!
//! The helpers return owned `Vec`s of arguments; callers assemble the
//! `MockAuth` / `MockAuthInvoke` references against these buffers and
//! then pass `&[MockAuth]` to `ctrl.mock_auths(...)` or `env.mock_auths(...)`.
//!
//! Rationale for the shape: `MockAuthInvoke` takes borrowed references
//! (`&'a Vec<Val>`), so the caller must own the argument vectors. Rather
//! than returning ready-to-use `MockAuth` trees (which would tangle
//! lifetimes), these helpers produce the *ingredients* and the tests
//! wire them together in-place.

use soroban_sdk::{Address, Env, IntoVal, Val, Vec};

/// Arguments needed to authorize a top-level `Controller::flash_loan` call.
///
/// The returned `args` vector is the argument list for
/// `flash_loan(caller, asset, amount, receiver, data)`. Usage:
///
/// ```ignore
/// let args = flash_loan_args(&env, &caller, &asset, amount, &receiver);
/// let invoke = MockAuthInvoke {
///     contract: &controller,
///     fn_name: "flash_loan",
///     args,
///     sub_invokes: &[], // token mint happens inside receiver; mock_all_auths
///                       // at the env level still covers nested mints when
///                       // the test also calls env.mock_all_auths().
/// };
/// let tree = [MockAuth { address: &caller, invoke: &invoke }];
/// ctrl.mock_auths(&tree).flash_loan(...);
/// ```
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

/// Arguments for `Controller::multiply(caller, account_id, e_mode, collateral,
/// debt, debt_token, mode, steps, initial_payment, convert_steps)`.
#[allow(clippy::too_many_arguments)]
pub fn multiply_args(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    collateral_token: &Address,
    debt_amount: i128,
    debt_token: &Address,
    mode: common::types::PositionMode,
    steps: &common::types::AggregatorSwap,
) -> Vec<Val> {
    (
        caller.clone(),
        account_id,
        e_mode_category,
        collateral_token.clone(),
        debt_amount,
        debt_token.clone(),
        mode,
        steps.clone(),
        None::<(Address, i128)>,
        None::<common::types::AggregatorSwap>,
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
    steps: &common::types::AggregatorSwap,
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
    steps: &common::types::AggregatorSwap,
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
    steps: &common::types::AggregatorSwap,
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
