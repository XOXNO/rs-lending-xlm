//! Swaps collateral between hub markets.

use common::errors::{CollateralError, GenericError};
use common::types::{Account, AccountPosition, AccountPositionType, HubAssetKey, StrategySwap};
use soroban_sdk::{assert_with_error, contractimpl, vec, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::account;
use crate::context::Cache;
use crate::events;
use crate::positions::get_supply_position_or_panic;
use crate::strategies::{
    prefetch_strategy_prices, strategy_finalize, swap_tokens_or_passthrough,
    withdraw_collateral_to_controller, StrategyWithdraw,
};
use crate::{
    positions::supply, risk::validation, spoke, storage, Controller, ControllerArgs,
    ControllerClient,
};

pub(crate) struct SwapCollateralParams<'a> {
    pub account_id: u64,
    pub current: &'a HubAssetKey,
    pub from_amount: i128,
    pub new: &'a HubAssetKey,
    pub swap: &'a StrategySwap,
}

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn swap_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        current: HubAssetKey,
        amount: i128,
        new: HubAssetKey,
        swap: Bytes,
    ) {
        process_swap_collateral(
            &env,
            &caller,
            SwapCollateralParams {
                account_id,
                current: &current,
                from_amount: amount,
                new: &new,
                swap: &swap,
            },
        );
    }
}

/// Withdraw collateral → swap → deposit replacement (debt-neutral until finalize).
pub(crate) fn process_swap_collateral(
    env: &Env,
    caller: &Address,
    params: SwapCollateralParams<'_>,
) {
    let SwapCollateralParams {
        account_id,
        current,
        from_amount,
        new,
        swap,
    } = params;

    caller.require_auth();
    validation::require_not_flash_loaning(env);

    // Reject identical (hub, asset); same token across hubs is passthrough.
    assert_with_error!(env, current != new, GenericError::AssetsAreTheSame);
    validation::require_hub_active(env, current.hub_id);
    validation::require_positive_amount(env, from_amount);

    let mut account = storage::get_account(env, account_id);
    account::require_owner_or_delegate(env, account_id, caller, &account.owner);
    let mut cache = Cache::new(env);
    validate_swap_new_collateral_preflight(env, &mut cache, &account, new);

    let extra_assets = vec![env, current.asset.clone(), new.asset.clone()];
    prefetch_strategy_prices(&mut cache, &account, &extra_assets);

    let current_pos: AccountPosition = get_supply_position_or_panic(env, &account, current);

    let actual_withdrawn = withdraw_collateral_to_controller(
        env,
        &mut account,
        &mut cache,
        StrategyWithdraw {
            hub_asset: current,
            amount: from_amount,
            position: &current_pos,
            action: events::PositionAction::SwColWd,
        },
    );

    // Passthrough when same asset (cross-hub).
    let swapped_amount = swap_tokens_or_passthrough(
        env,
        caller,
        &current.asset,
        actual_withdrawn,
        &new.asset,
        swap,
    );

    let deposit_assets = vec![env, (new.clone(), swapped_amount)];
    supply::process_deposit(
        env,
        &env.current_contract_address(),
        &mut account,
        &deposit_assets,
        &mut cache,
    );

    strategy_finalize(env, account_id, &account, &mut cache);
}

pub(crate) fn validate_swap_new_collateral_preflight(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    new: &HubAssetKey,
) {
    let config = spoke::require_listed_active_config(env, cache, account.spoke_id, new);

    assert_with_error!(env, config.can_supply(), CollateralError::NotCollateral);

    if !account.supply_positions.contains_key(new.clone()) {
        let new_assets = vec![env, (new.clone(), 0i128)];
        validation::validate_bulk_position_limits(
            env,
            account,
            AccountPositionType::Deposit,
            &new_assets,
        );
    }
}
