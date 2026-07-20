use crate::types::{HubAssetKey, Payment, PositionMode, StrategySwap};
use cvlr::nondet::nondet;
use cvlr_soroban::nondet_address;
use soroban_sdk::{vec, Address, Env, Vec};

/// Hub-0 coordinate for `asset`.
fn hub0(asset: Address) -> HubAssetKey {
    HubAssetKey { hub_id: 0, asset }
}

/// Single-asset supply shim for `Controller::supply` (spoke 0).
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
        vec![&env, (hub0(asset), amount)],
    )
}

/// Single-asset borrow shim. Havocs `to` (self vs recipient); debt/health identical either way.
pub fn borrow_single(env: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    let to: Option<Address> = if nondet() {
        Some(nondet_address())
    } else {
        None
    };
    crate::Controller::borrow(
        env.clone(),
        caller,
        account_id,
        vec![&env, (hub0(asset), amount)],
        to,
    );
}

/// Single-asset withdraw shim. Havocs `to` (self vs recipient); position/health identical either way.
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
        vec![&env, (hub0(asset), amount)],
        to,
    );
}

/// Single-asset repay shim for `Controller::repay`.
pub fn repay_single(env: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    crate::Controller::repay(
        env.clone(),
        caller,
        account_id,
        vec![&env, (hub0(asset), amount)],
    );
}

/// Full `Controller::multiply` shim; havoced optional parameters explore all branches.
pub fn multiply(
    env: Env,
    caller: Address,
    spoke_id: u32,
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
    let initial_payment: Option<(HubAssetKey, i128)> = if take_initial {
        let initial_amount: i128 = nondet();
        Some((hub0(nondet_address()), initial_amount))
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
        spoke_id,
        hub0(collateral_token),
        debt_to_flash_loan,
        hub0(debt_token),
        mode,
        steps,
        initial_payment,
        convert_steps,
    )
}

/// Minimal `multiply` shim for early-exit negative-path rules.
pub fn multiply_minimal(
    env: Env,
    caller: Address,
    spoke_id: u32,
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
        spoke_id,
        hub0(collateral_token),
        debt_to_flash_loan,
        hub0(debt_token),
        mode,
        steps,
        None,
        None,
    )
}

/// `repay_debt_with_collateral` shim with `close_position = false`.
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
        hub0(collateral_token),
        collateral_amount,
        hub0(debt_token),
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
        hub0(collateral_token),
        collateral_amount,
        hub0(debt_token),
        steps,
        true,
    );
}

/// `Controller::liquidate` shim; lifts asset-keyed payments onto hub 0.
pub fn liquidate(env: Env, liquidator: Address, account_id: u64, debt_payments: Vec<Payment>) {
    let mut hub_payments: Vec<(HubAssetKey, i128)> = Vec::new(&env);
    for (asset, amount) in debt_payments.iter() {
        hub_payments.push_back((hub0(asset), amount));
    }
    crate::Controller::liquidate(env, liquidator, account_id, hub_payments);
}
