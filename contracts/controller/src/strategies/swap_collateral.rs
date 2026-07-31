use common::errors::{CollateralError, GenericError};
use common::types::{Account, AccountPosition, HubAssetKey, StrategySwap};
use common::validation::require_positive_amount;
use soroban_sdk::{assert_with_error, vec, Address, Env};

use crate::account;
use crate::config;
use crate::context::Cache;
use crate::events;
use crate::positions::get_supply_position_or_panic;
use crate::strategies::{
    prefetch_strategy_prices, strategy_finalize, swap_tokens_or_passthrough,
    withdraw_collateral_to_controller, StrategyWithdraw,
};
use crate::{positions::supply, risk::validation, storage};

pub(crate) struct SwapCollateralParams<'a> {
    pub account_id: u64,
    pub current: &'a HubAssetKey,
    pub from_amount: i128,
    pub new: &'a HubAssetKey,
    pub swap: &'a StrategySwap,
}

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

    assert_with_error!(env, current != new, GenericError::AssetsAreTheSame);
    config::require_hub_active(env, current.hub_id);
    require_positive_amount(env, from_amount);

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

    strategy_finalize(env, account_id, &mut account, &mut cache);
}

pub(crate) fn validate_swap_new_collateral_preflight(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    new: &HubAssetKey,
) {
    let config = cache.require_listed_active_config(account.spoke_id, new);

    assert_with_error!(env, config.can_supply(), CollateralError::NotCollateral);
}
