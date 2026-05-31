use common::types::{Payment, PositionMode, SwapSteps};
use cvlr::nondet::nondet;
use cvlr_soroban::nondet_address;
use soroban_sdk::{vec, Address, Env, Vec};

pub fn supply_single(
    env: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) -> u64 {
    crate::Controller::supply(
        env.clone(),
        caller,
        account_id,
        0,
        vec![&env, (asset, amount)],
    )
}

pub fn borrow_single(env: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    crate::Controller::borrow(env.clone(), caller, account_id, vec![&env, (asset, amount)]);
}

pub fn withdraw_single(env: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    crate::Controller::withdraw(env.clone(), caller, account_id, vec![&env, (asset, amount)]);
}

pub fn repay_single(env: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    crate::Controller::repay(env.clone(), caller, account_id, vec![&env, (asset, amount)]);
}

/// Public-API shim for `Controller::multiply` (`controller/src/strategy.rs:41-67`).
///
/// Existing rule signatures pin `caller`, `e_mode_category`, `collateral_token`,
/// `debt_to_flash_loan`, `debt_token`, `mode`, `steps`. The remaining production
/// parameters -- `account_id` (0 = create vs >0 = load existing), `initial_payment`
/// (None vs Some), `convert_steps` (None vs Some) -- are havoced inside the shim
/// so all production branches stay reachable to the prover.
pub fn multiply(
    env: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: SwapSteps,
) -> u64 {
    let mode = match mode {
        0 => PositionMode::Normal,
        1 => PositionMode::Multiply,
        2 => PositionMode::Long,
        3 => PositionMode::Short,
        _ => panic!("invalid strategy mode for certora compat"),
    };

    // Havoc account_id so both the create-new (== 0) and load-existing (> 0)
    // branches in `process_multiply` are explored.
    let account_id: u64 = nondet();

    // Havoc `initial_payment` across {None, Some(asset, amount)}. SwapSteps has
    // no Nondet impl, so reuse the rule-provided `steps` for `convert_steps`.
    let take_initial: bool = nondet();
    let initial_payment: Option<Payment> = if take_initial {
        let initial_amount: i128 = nondet();
        Some((nondet_address(), initial_amount))
    } else {
        None
    };
    let take_convert: bool = nondet();
    let convert_steps: Option<SwapSteps> = if take_convert {
        Some(steps.clone())
    } else {
        None
    };

    crate::Controller::multiply(
        env,
        caller,
        account_id,
        e_mode_category,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        mode,
        steps,
        initial_payment,
        convert_steps,
    )
}

/// Public-API shim for `Controller::repay_debt_with_collateral`
/// (`controller/src/strategy.rs:112-132`). `close_position` is havoced so the
/// account-deletion branch in `process_repay_debt_with_collateral` is reachable.
pub fn repay_debt_with_collateral(
    env: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    steps: SwapSteps,
) {
    let close_position: bool = nondet();
    crate::Controller::repay_debt_with_collateral(
        env,
        caller,
        account_id,
        collateral_token,
        collateral_amount,
        debt_token,
        steps,
        close_position,
    );
}

/// Minimal-mode shim for `Controller::multiply` used by negative-path rules
/// (e.g. `multiply_rejects_same_tokens`, `multiply_requires_collateralizable`)
/// where the panic fires inside `process_multiply` *before* any of the optional
/// branches matter. Pinning `account_id = 0`, `initial_payment = None` and
/// `convert_steps = None` removes three nondet draws and the 4-way payment-token
/// branch from the prover's exploration with no loss of property coverage,
/// because the early-exit panic is reached on every path.
pub fn multiply_minimal(
    env: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: SwapSteps,
) -> u64 {
    let mode = match mode {
        0 => PositionMode::Normal,
        1 => PositionMode::Multiply,
        2 => PositionMode::Long,
        3 => PositionMode::Short,
        _ => panic!("invalid strategy mode for certora compat"),
    };

    crate::Controller::multiply(
        env,
        caller,
        0, // create new account
        e_mode_category,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        mode,
        steps,
        None, // no initial_payment
        None, // no convert_steps
    )
}

/// Minimal-mode shim for `Controller::repay_debt_with_collateral` used by
/// flash-loan-guard rules where the early-exit panic fires before the
/// `close_position` branch is consulted. Pins `close_position = false` to
/// remove the unbounded `execute_withdraw_all` loop from the prover's path.
pub fn repay_debt_with_collateral_minimal(
    env: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    steps: SwapSteps,
) {
    crate::Controller::repay_debt_with_collateral(
        env,
        caller,
        account_id,
        collateral_token,
        collateral_amount,
        debt_token,
        steps,
        false, // close_position pinned off
    );
}

/// Variant of `repay_debt_with_collateral` that pins `close_position = true`.
/// Used by the dedicated full-close rule that verifies the
/// `execute_withdraw_all` + account-deletion branch in
/// `process_repay_debt_with_collateral`.
pub fn repay_debt_with_collateral_close(
    env: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    steps: SwapSteps,
) {
    crate::Controller::repay_debt_with_collateral(
        env,
        caller,
        account_id,
        collateral_token,
        collateral_amount,
        debt_token,
        steps,
        true, // close_position pinned on
    );
}

/// Variant of `multiply` that pins `account_id = 0` and `initial_payment = None`
/// while leaving `convert_steps` controlled by the rule. Used by the
/// canonical-happy-path multiply rule.
pub fn multiply_basic(
    env: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: SwapSteps,
) -> u64 {
    let mode = match mode {
        0 => PositionMode::Normal,
        1 => PositionMode::Multiply,
        2 => PositionMode::Long,
        3 => PositionMode::Short,
        _ => panic!("invalid strategy mode for certora compat"),
    };

    crate::Controller::multiply(
        env,
        caller,
        0,
        e_mode_category,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        mode,
        steps,
        None,
        None,
    )
}

/// Variant of `multiply` with a pinned initial payment in `collateral_token`
/// (the cheap branch in `collect_initial_multiply_payment` that skips the
/// nested `swap_tokens`).
pub fn multiply_with_initial_payment_collateral(
    env: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: SwapSteps,
    initial_amount: i128,
) -> u64 {
    let mode = match mode {
        0 => PositionMode::Normal,
        1 => PositionMode::Multiply,
        2 => PositionMode::Long,
        3 => PositionMode::Short,
        _ => panic!("invalid strategy mode for certora compat"),
    };

    let initial_payment: Option<Payment> = Some((collateral_token.clone(), initial_amount));
    crate::Controller::multiply(
        env,
        caller,
        0,
        e_mode_category,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        mode,
        steps,
        initial_payment,
        None,
    )
}

/// Variant of `multiply` with a pinned initial payment in a third token
/// (neither collateral nor debt) and `convert_steps` provided. Exercises the
/// nested `swap_tokens` branch.
pub fn multiply_with_initial_payment_third_token(
    env: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: SwapSteps,
    third_token: Address,
    initial_amount: i128,
    convert_steps: SwapSteps,
) -> u64 {
    let mode = match mode {
        0 => PositionMode::Normal,
        1 => PositionMode::Multiply,
        2 => PositionMode::Long,
        3 => PositionMode::Short,
        _ => panic!("invalid strategy mode for certora compat"),
    };

    let initial_payment: Option<Payment> = Some((third_token, initial_amount));
    crate::Controller::multiply(
        env,
        caller,
        0,
        e_mode_category,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        mode,
        steps,
        initial_payment,
        Some(convert_steps),
    )
}

/// Public-API shim for `Controller::liquidate`
/// (`controller/src/positions/liquidation.rs:17-19`). Routes through the
/// `#[when_not_paused]` + `liquidator.require_auth()` path so liquidation
/// rules exercise the public entry point instead of `process_liquidation`
/// directly.
pub fn liquidate(env: Env, liquidator: Address, account_id: u64, debt_payments: Vec<Payment>) {
    crate::Controller::liquidate(env, liquidator, account_id, debt_payments);
}
