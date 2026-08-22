use common::errors::GenericError;
use common::types::{HubAssetKey, StrategySwap};
use common::validation::require_positive_amount;
use soroban_sdk::{assert_with_error, vec, Address, Env};

use crate::account;
use crate::config;
use crate::context::Cache;
use crate::events;
use crate::positions::require_can_supply;
use crate::risk::validation::require_authorized_caller;
use crate::strategies::{
    prefetch_strategy_prices, strategy_finalize, withdraw_and_swap_from_supply,
};
use crate::{positions::supply, storage};

pub(crate) struct SwapCollateralParams<'a> {
    pub account_id: u64,
    pub current: &'a HubAssetKey,
    pub from_amount: i128,
    pub new: &'a HubAssetKey,
    pub swap: &'a StrategySwap,
}

/// Converts `from_amount` of the account's `current` collateral into `new`
/// collateral: withdraws and swaps the existing supply position, deposits the
/// resulting output as `new` collateral, and finalizes the account. Requires
/// the caller to be the account owner or a delegate, `current` and `new` to
/// differ, and `current`'s hub to be active.
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

    require_authorized_caller(env, caller);

    assert_with_error!(env, current != new, GenericError::AssetsAreTheSame);
    config::require_hub_active(env, current.hub_id);
    require_positive_amount(env, from_amount);

    let mut account = storage::get_account(env, account_id);
    account::require_owner_or_delegate(env, account_id, caller, &account.owner);
    let mut cache = Cache::new(env);
    // Preflight: `new` must be suppliable into the account's spoke.
    require_can_supply(env, &mut cache, account.spoke_id, new);

    let extra_assets = vec![env, current.asset.clone(), new.asset.clone()];
    prefetch_strategy_prices(&mut cache, &account, &extra_assets);

    let swapped_amount = withdraw_and_swap_from_supply(
        env,
        &mut account,
        &mut cache,
        caller,
        current,
        from_amount,
        &new.asset,
        swap,
        events::PositionAction::SwColWd,
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
