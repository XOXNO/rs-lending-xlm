use common::types::{PositionMode, SwapSteps};
use soroban_sdk::{vec, Address, Env};

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

pub fn repay_debt_with_collateral(
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
        false,
    );
}
