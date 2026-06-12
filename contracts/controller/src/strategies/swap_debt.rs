use common::errors::{CollateralError, GenericError};
use controller_interface::types::{DebtPosition, StrategySwap};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Bytes, Env, Vec};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::oracle::policy::OraclePolicy;
use crate::strategies::helpers::{
    open_strategy_borrow, repay_debt_from_controller, strategy_finalize, swap_tokens, StrategyRepay,
};
use crate::{helpers::utils, storage, validation, Controller, ControllerArgs, ControllerClient};

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
            account_id,
            &existing_debt_token,
            amount,
            &new_debt_token,
            &swap,
        );
    }
}

pub fn process_swap_debt(
    env: &Env,
    caller: &Address,
    account_id: u64,
    existing_debt_token: &Address,
    new_debt_amount: i128,
    new_debt_token: &Address,
    swap: &StrategySwap,
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
    let mut cache = Cache::new(env, OraclePolicy::RiskIncreasing);

    validation::require_positive_amount(env, new_debt_amount);

    // Siloed debt cannot be swapped into or out of a multi-asset debt set.
    let existing_debt_config = cache.cached_asset_config(existing_debt_token);
    let new_debt_config = cache.cached_asset_config(new_debt_token);
    if existing_debt_config.is_siloed_borrowing || new_debt_config.is_siloed_borrowing {
        panic_with_error!(env, CollateralError::NotBorrowableSiloed);
    }

    // Bulk-prefetch all RedStone feeds for this tx before the first price read.
    // Universe: existing supply + borrow positions (required for the post-swap
    // LTV/HF checks in strategy_finalize) plus both debt tokens.
    let mut prefetch_assets: Vec<Address> = account.supply_positions.keys();
    prefetch_assets.append(&account.borrow_positions.keys());
    utils::push_unique_address(&mut prefetch_assets, existing_debt_token.clone());
    utils::push_unique_address(&mut prefetch_assets, new_debt_token.clone());
    crate::oracle::prefetch_redstone_feeds(&mut cache, &prefetch_assets);

    let amount_received = open_strategy_borrow(
        env,
        &mut cache,
        &mut account,
        account_id,
        new_debt_token,
        new_debt_amount,
    );

    let swapped_amount = swap_tokens(
        env,
        new_debt_token,
        amount_received,
        existing_debt_token,
        swap,
    );

    let existing_pos: DebtPosition = (&account
        .borrow_positions
        .get(existing_debt_token.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound)))
        .into();

    repay_debt_from_controller(
        env,
        &mut account,
        account_id,
        &mut cache,
        caller,
        StrategyRepay {
            debt_token: existing_debt_token,
            debt_available: swapped_amount,
            debt_pos: &existing_pos,
            action: crate::events::PositionAction::SwDebtR,
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
