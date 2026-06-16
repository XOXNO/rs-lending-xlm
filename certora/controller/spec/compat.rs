use crate::types::{Payment, PositionMode, StrategySwap};
use cvlr::nondet::nondet;
use cvlr_soroban::nondet_address;
use soroban_sdk::{vec, Address, Env, Vec};

/// Single-asset supply shim for `Controller::supply`.
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

/// Single-asset borrow shim for `Controller::borrow`.
pub fn borrow_single(env: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    crate::Controller::borrow(env.clone(), caller, account_id, vec![&env, (asset, amount)]);
}

/// Single-asset withdraw shim. Havocs the `to` recipient so rules cover both
/// withdraw-to-self (`None`) and withdraw-to-recipient (`Some`) branches; the
/// account's position and health math is identical either way.
pub fn withdraw_single(env: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    let to: Option<Address> = if nondet() {
        Some(nondet_address())
    } else {
        None
    };
    crate::Controller::withdraw(
        env.clone(),
        caller,
        account_id,
        vec![&env, (asset, amount)],
        to,
    );
}

/// Single-asset repay shim for `Controller::repay`.
pub fn repay_single(env: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    crate::Controller::repay(env.clone(), caller, account_id, vec![&env, (asset, amount)]);
}

/// Full `Controller::multiply` shim; havoced optional parameters explore all branches.
pub fn multiply(
    env: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: StrategySwap,
) -> u64 {
    let mode = match mode {
        0 => PositionMode::Normal,
        1 => PositionMode::Multiply,
        2 => PositionMode::Long,
        3 => PositionMode::Short,
        _ => panic!("invalid strategy mode for certora compat"),
    };

    let account_id: u64 = nondet();

    let take_initial: bool = nondet();
    let initial_payment: Option<Payment> = if take_initial {
        let initial_amount: i128 = nondet();
        Some((nondet_address(), initial_amount))
    } else {
        None
    };
    let take_convert: bool = nondet();
    let convert_steps: Option<StrategySwap> = if take_convert {
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

/// `Controller::repay_debt_with_collateral` shim; havoced `close_position` covers both branches.
pub fn repay_debt_with_collateral(
    env: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    steps: StrategySwap,
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

/// Minimal `multiply` shim for early-exit negative-path rules.
pub fn multiply_minimal(
    env: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: StrategySwap,
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

/// Minimal `repay_debt_with_collateral` shim with `close_position = false`.
pub fn repay_debt_with_collateral_minimal(
    env: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    steps: StrategySwap,
) {
    crate::Controller::repay_debt_with_collateral(
        env,
        caller,
        account_id,
        collateral_token,
        collateral_amount,
        debt_token,
        steps,
        false,
    );
}

/// `repay_debt_with_collateral` shim with `close_position = true`.
pub fn repay_debt_with_collateral_close(
    env: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    steps: StrategySwap,
) {
    crate::Controller::repay_debt_with_collateral(
        env,
        caller,
        account_id,
        collateral_token,
        collateral_amount,
        debt_token,
        steps,
        true,
    );
}

/// `multiply` shim with new account and no initial payment.
pub fn multiply_basic(
    env: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: StrategySwap,
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

/// `multiply` shim with initial payment in `collateral_token`.
pub fn multiply_with_initial_payment_collateral(
    env: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: StrategySwap,
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

/// `multiply` shim with initial payment in a third token and `convert_steps`.
pub fn multiply_with_initial_payment_third_token(
    env: Env,
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

/// Public `Controller::liquidate` entry point shim.
pub fn liquidate(env: Env, liquidator: Address, account_id: u64, debt_payments: Vec<Payment>) {
    crate::Controller::liquidate(env, liquidator, account_id, debt_payments);
}
