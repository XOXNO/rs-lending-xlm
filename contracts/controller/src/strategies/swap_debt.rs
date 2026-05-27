use common::errors::{CollateralError, GenericError};
use common::types::{AggregatorSwap, DebtPosition};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, symbol_short, Address, Env};
use stellar_macros::when_not_paused;

use crate::cache::ControllerCache;
use crate::oracle::policy::OraclePolicy;
use crate::strategies::helpers::{
    open_strategy_borrow, repay_debt_from_controller, strategy_finalize, swap_tokens, StrategyRepay,
};
use crate::{storage, validation, Controller, ControllerArgs, ControllerClient};

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
        swap: AggregatorSwap,
    ) {
        process_swap_debt(
            &env,
            &caller,
            account_id,
            &existing_debt_token,
            amount,
            &new_debt_token,
            &swap,
        );
    }
}

// Swaps debt position to new token.
pub fn process_swap_debt(
    env: &Env,
    caller: &Address,
    account_id: u64,
    existing_debt_token: &Address,
    new_debt_amount: i128,
    new_debt_token: &Address,
    swap: &AggregatorSwap,
) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    assert_with_error!(
        env,
        existing_debt_token != new_debt_token,
        GenericError::AssetsAreTheSame
    );

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);

    // Strategy borrows are risk-increasing.
    let mut cache = ControllerCache::new(env, OraclePolicy::RiskIncreasing);

    validation::require_amount_positive(env, new_debt_amount);
    // Reject zero-floor swap requests at entry.
    validation::require_amount_positive(env, swap.total_min_out);

    // Siloed debt cannot be swapped into or out of a multi-asset debt set.
    let existing_debt_config = cache.cached_asset_config(existing_debt_token);
    let new_debt_config = cache.cached_asset_config(new_debt_token);
    if existing_debt_config.is_siloed_borrowing || new_debt_config.is_siloed_borrowing {
        panic_with_error!(env, CollateralError::NotBorrowableSiloed);
    }

    let amount_received = open_strategy_borrow(
        env,
        &mut cache,
        &mut account,
        new_debt_token,
        new_debt_amount,
    );

    let swapped_amount = swap_tokens(
        env,
        new_debt_token,
        amount_received,
        existing_debt_token,
        swap,
        caller,
    );

    let existing_pos: DebtPosition = (&account
        .borrow_positions
        .get(existing_debt_token.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound)))
        .into();

    repay_debt_from_controller(
        env,
        &mut account,
        &mut cache,
        caller,
        StrategyRepay {
            debt_token: existing_debt_token,
            debt_available: swapped_amount,
            debt_pos: &existing_pos,
            action: symbol_short!("sw_debt_r"),
        },
    );

    strategy_finalize(
        env,
        account_id,
        &mut account,
        &mut cache,
        crate::strategies::helpers::StrategyTouched {
            supply_assets: &[],
            borrow_assets: &[existing_debt_token, new_debt_token],
        },
    );
}
