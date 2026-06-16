//! Debt swap strategy.
//!
//! Pipeline: auth → flash guard → account → cache → preflight → prefetch →
//! borrow → swap → repay → `strategy_finalize`.

use common::errors::{CollateralError, GenericError};
use controller_interface::types::{Account, DebtPosition, StrategySwap};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::oracle::policy::OraclePolicy;
use crate::strategies::{
    open_strategy_borrow, prefetch_strategy_oracles, repay_debt_from_controller, strategy_finalize,
    swap_tokens, StrategyRepay,
};
use crate::{storage, validation, Controller, ControllerArgs, ControllerClient};

/// Parameters for `process_swap_debt`.
pub struct SwapDebtParams<'a> {
    pub account_id: u64,
    pub existing_debt_token: &'a Address,
    pub new_debt_amount: i128,
    pub new_debt_token: &'a Address,
    pub swap: &'a StrategySwap,
}

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn swap_debt(
        env: Env,
        caller: Address,
        account_id: u64,
        existing_debt_token: Address,
        amount: i128,
        new_debt_token: Address,
        swap: Bytes,
    ) {
        process_swap_debt(
            &env,
            &caller,
            SwapDebtParams {
                account_id,
                existing_debt_token: &existing_debt_token,
                new_debt_amount: amount,
                new_debt_token: &new_debt_token,
                swap: &swap,
            },
        );
    }
}

pub fn process_swap_debt(env: &Env, caller: &Address, params: SwapDebtParams<'_>) {
    let SwapDebtParams {
        account_id,
        existing_debt_token,
        new_debt_amount,
        new_debt_token,
        swap,
    } = params;

    caller.require_auth();
    validation::require_not_flash_loaning(env);

    assert_with_error!(
        env,
        existing_debt_token != new_debt_token,
        GenericError::AssetsAreTheSame
    );

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);

    let mut cache = Cache::new(env, OraclePolicy::RiskIncreasing);

    validation::require_positive_amount(env, new_debt_amount);

    let existing_pos =
        load_existing_debt_position(env, &account, existing_debt_token);

    let extra_assets = soroban_sdk::vec![env, existing_debt_token.clone(), new_debt_token.clone()];
    prefetch_strategy_oracles(&mut cache, &account, &extra_assets);

    let amount_received = open_strategy_borrow(
        env,
        &mut cache,
        &mut account,
        new_debt_token,
        new_debt_amount,
    );

    let swapped_amount = swap_tokens(
        env,
        caller,
        new_debt_token,
        amount_received,
        existing_debt_token,
        swap,
    );

    repay_debt_from_controller(
        env,
        &mut account,
        &mut cache,
        caller,
        StrategyRepay {
            debt_token: existing_debt_token,
            debt_available: swapped_amount,
            debt_pos: &existing_pos,
            action: crate::events::PositionAction::SwDebtR,
        },
    );

    strategy_finalize(env, account_id, &mut account, &mut cache);
}

fn load_existing_debt_position(
    env: &Env,
    account: &Account,
    existing_debt_token: &Address,
) -> DebtPosition {
    account
        .borrow_positions
        .get(existing_debt_token.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound))
        .into()
}
