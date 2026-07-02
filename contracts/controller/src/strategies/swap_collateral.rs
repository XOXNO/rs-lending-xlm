//! Swaps collateral between hub markets.

use common::errors::{CollateralError, GenericError};
use common::types::{Account, AccountPosition, AccountPositionType, HubAssetKey, StrategySwap};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::context::Cache;
use crate::events;
use crate::strategies::{
    prefetch_strategy_oracles, strategy_finalize, swap_tokens, withdraw_collateral_to_controller,
    StrategyWithdraw,
};
use crate::{
    positions::supply, risk::validation, spoke, storage, Controller, ControllerArgs,
    ControllerClient,
};

pub struct SwapCollateralParams<'a> {
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

pub fn process_swap_collateral(env: &Env, caller: &Address, params: SwapCollateralParams<'_>) {
    let SwapCollateralParams {
        account_id,
        current,
        from_amount,
        new,
        swap,
    } = params;

    caller.require_auth();
    validation::require_not_flash_loaning(env);

    // The swap leg needs distinct underlying tokens.
    assert_with_error!(
        env,
        current.asset != new.asset,
        GenericError::AssetsAreTheSame
    );

    // The withdraw leg settles on `current`'s hub; `new`'s hub is gated by the
    // deposit's `require_hub_active`.
    validation::require_hub_active(env, current.hub_id);

    let mut account = storage::get_account(env, account_id);
    crate::account::require_owner_or_delegate(env, account_id, caller, &account.owner);

    let mut cache = Cache::new(env);

    validation::require_positive_amount(env, from_amount);

    validate_swap_new_collateral_preflight(env, &mut cache, &account, new);

    let extra_assets = soroban_sdk::vec![env, current.asset.clone(), new.asset.clone()];
    prefetch_strategy_oracles(&mut cache, &account, &extra_assets);

    let current_pos: AccountPosition = (&account
        .supply_positions
        .get(current.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::CollateralPositionNotFound)))
        .into();

    // D{current_collateral.decimals}{Token(current_collateral)} withdrawal request to balance delta.
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

    // D{current_collateral.decimals}{Token(current_collateral)} -> Token(new_collateral).
    let swapped_amount = swap_tokens(
        env,
        caller,
        &current.asset,
        actual_withdrawn,
        &new.asset,
        swap,
    );

    // D{new_collateral.decimals}{Token(new_collateral)} deposited as replacement collateral.
    let deposit_assets = soroban_sdk::vec![env, (new.clone(), swapped_amount)];
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
    new: &HubAssetKey,
) {
    let config = spoke::require_listed_active_config(env, cache, account.spoke_id, new);

    assert_with_error!(env, config.can_supply(), CollateralError::NotCollateral);

    if !account.supply_positions.contains_key(new.clone()) {
        let new_assets = soroban_sdk::vec![env, (new.clone(), 0i128)];
        validation::validate_bulk_position_limits(
            env,
            account,
            AccountPositionType::Deposit,
            &new_assets,
        );
    }
}
