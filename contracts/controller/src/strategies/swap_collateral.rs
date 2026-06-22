//! Collateral swap strategy.
//!
//! Pipeline: auth → flash guard → account → cache(policy) → preflight →
//! prefetch → withdraw → swap → deposit → `strategy_finalize`.

use common::errors::{CollateralError, GenericError};
use controller_interface::types::{Account, AccountPosition, AccountPositionType, StrategySwap};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::events;
use crate::oracle::policy::OraclePolicy;
use crate::strategies::{
    prefetch_strategy_oracles, strategy_finalize, swap_tokens, withdraw_collateral_to_controller,
    StrategyWithdraw,
};
use crate::{
    emode, positions::supply, storage, validation, Controller, ControllerArgs, ControllerClient,
};

/// Parameters for `process_swap_collateral`.
pub struct SwapCollateralParams<'a> {
    pub account_id: u64,
    pub current_collateral: &'a Address,
    pub from_amount: i128,
    pub new_collateral: &'a Address,
    pub swap: &'a StrategySwap,
}

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn swap_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        current_collateral: Address,
        amount: i128,
        new_collateral: Address,
        swap: Bytes,
    ) {
        process_swap_collateral(
            &env,
            &caller,
            SwapCollateralParams {
                account_id,
                current_collateral: &current_collateral,
                from_amount: amount,
                new_collateral: &new_collateral,
                swap: &swap,
            },
        );
    }
}

pub fn process_swap_collateral(env: &Env, caller: &Address, params: SwapCollateralParams<'_>) {
    let SwapCollateralParams {
        account_id,
        current_collateral,
        from_amount,
        new_collateral,
        swap,
    } = params;

    caller.require_auth();
    validation::require_not_flash_loaning(env);

    assert_with_error!(
        env,
        current_collateral != new_collateral,
        GenericError::AssetsAreTheSame
    );

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);

    let policy = if account.borrow_positions.is_empty() {
        OraclePolicy::RiskDecreasing
    } else {
        OraclePolicy::RiskIncreasing
    };
    let mut cache = Cache::new(env, policy);

    validation::require_positive_amount(env, from_amount);

    validate_swap_new_collateral_preflight(env, &mut cache, &account, new_collateral);

    let extra_assets = soroban_sdk::vec![env, current_collateral.clone(), new_collateral.clone()];
    prefetch_strategy_oracles(&mut cache, &account, &extra_assets);

    let current_pos: AccountPosition = (&account
        .supply_positions
        .get(current_collateral.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::CollateralPositionNotFound)))
        .into();

    let actual_withdrawn = withdraw_collateral_to_controller(
        env,
        &mut account,
        &mut cache,
        StrategyWithdraw {
            asset: current_collateral,
            amount: from_amount,
            position: &current_pos,
            action: events::PositionAction::SwColWd,
        },
    );

    let swapped_amount = swap_tokens(
        env,
        caller,
        current_collateral,
        actual_withdrawn,
        new_collateral,
        swap,
    );

    let deposit_assets = soroban_sdk::vec![env, (new_collateral.clone(), swapped_amount)];
    supply::process_deposit(
        env,
        &env.current_contract_address(),
        &mut account,
        &deposit_assets,
        &mut cache,
    );

    strategy_finalize(env, account_id, &mut account, &mut cache);
}

/// Rejects replacement collateral that cannot be supplied after the swap.
pub(crate) fn validate_swap_new_collateral_preflight(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    new_collateral: &Address,
) {
    let e_mode = cache.active_e_mode_category(env, account.e_mode_category_id);
    let config = emode::effective_asset_config(env, account, new_collateral, cache, &e_mode);
    emode::validate_e_mode_asset(env, cache, account.e_mode_category_id, new_collateral);

    assert_with_error!(env, config.can_supply(), CollateralError::NotCollateral);

    if !account
        .supply_positions
        .contains_key(new_collateral.clone())
    {
        let new_assets = soroban_sdk::vec![env, (new_collateral.clone(), 0i128)];
        validation::validate_bulk_position_limits(
            env,
            account,
            AccountPositionType::Deposit,
            &new_assets,
        );
    }
}
