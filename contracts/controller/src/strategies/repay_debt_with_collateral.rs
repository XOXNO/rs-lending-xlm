use common::errors::CollateralError;
use controller_interface::types::{Account, AccountPosition, DebtPosition, StrategySwap};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Bytes, Env, Vec};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::oracle::policy::OraclePolicy;
use crate::strategies::helpers::{
    execute_withdraw_all, repay_debt_from_controller, strategy_finalize, swap_tokens,
    withdraw_collateral_to_controller, StrategyRepay, StrategyWithdraw,
};
use crate::{helpers::utils, storage, validation, Controller, ControllerArgs, ControllerClient};

/// Parameters for `process_repay_debt_with_collateral`.
pub struct RepayWithCollateralParams<'a> {
    pub account_id: u64,
    pub collateral_token: &'a Address,
    pub collateral_amount: i128,
    pub debt_token: &'a Address,
    pub swap: &'a StrategySwap,
    pub close_position: bool,
}

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn repay_debt_with_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        collateral_token: Address,
        collateral_amount: i128,
        debt_token: Address,
        swap: Bytes,
        close_position: bool,
    ) {
        process_repay_debt_with_collateral(
            &env,
            &caller,
            RepayWithCollateralParams {
                account_id,
                collateral_token: &collateral_token,
                collateral_amount,
                debt_token: &debt_token,
                swap: &swap,
                close_position,
            },
        );
    }
}

// Repays debt with swapped collateral.
pub fn process_repay_debt_with_collateral(
    env: &Env,
    caller: &Address,
    params: RepayWithCollateralParams<'_>,
) {
    let RepayWithCollateralParams {
        account_id,
        collateral_token,
        collateral_amount,
        debt_token,
        swap,
        close_position,
    } = params;

    caller.require_auth();
    validation::require_not_flash_loaning(env);
    validation::require_positive_amount(env, collateral_amount);

    // Same-asset flow is intentional: self-collateralized positions
    // (e.g. stablecoin/stablecoin) net both legs atomically, no aggregator route.

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);

    // RiskIncreasing policy: swap slippage can leave the post-repay HF lower.
    let mut cache = Cache::new(env, OraclePolicy::RiskIncreasing);

    let (collateral_pos, debt_pos) =
        load_repay_with_collateral_positions(env, &account, collateral_token, debt_token);

    // Bulk-prefetch all RedStone feeds for this tx before the first price read.
    // Universe: existing supply + borrow positions (required for the post-repay
    // LTV/HF checks in strategy_finalize) plus both strategy legs.
    let mut prefetch_assets: Vec<Address> = account.supply_positions.keys();
    prefetch_assets.append(&account.borrow_positions.keys());
    utils::push_unique_address(&mut prefetch_assets, collateral_token.clone());
    utils::push_unique_address(&mut prefetch_assets, debt_token.clone());
    crate::oracle::prefetch_redstone_feeds(&mut cache, &prefetch_assets);

    let actual_withdrawn = withdraw_collateral_to_controller(
        env,
        &mut account,
        &mut cache,
        StrategyWithdraw {
            asset: collateral_token,
            amount: collateral_amount,
            position: &collateral_pos,
            action: crate::events::PositionAction::RpColWd,
        },
    );

    let debt_available = swap_or_net_collateral_to_debt(
        env,
        caller,
        collateral_token,
        debt_token,
        actual_withdrawn,
        swap,
    );
    repay_debt_from_controller(
        env,
        &mut account,
        account_id,
        &mut cache,
        caller,
        StrategyRepay {
            debt_token,
            debt_available,
            debt_pos: &debt_pos,
            action: crate::events::PositionAction::RpColR,
        },
    );

    close_remaining_collateral_if_requested(env, &mut account, caller, &mut cache, close_position);

    strategy_finalize(
        env,
        account_id,
        &mut account,
        &mut cache,
        crate::strategies::helpers::StrategyTouched {
            supply_assets: &[collateral_token],
            borrow_assets: &[debt_token],
        },
    );
}

fn load_repay_with_collateral_positions(
    env: &Env,
    account: &Account,
    collateral_token: &Address,
    debt_token: &Address,
) -> (AccountPosition, DebtPosition) {
    // Validate both positions before moving tokens so a missing position
    // surfaces as its specific error, not a host panic on a later transfer.
    let collateral_pos = account
        .supply_positions
        .get(collateral_token.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::CollateralPositionNotFound));
    let debt_pos = account
        .borrow_positions
        .get(debt_token.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound));

    ((&collateral_pos).into(), (&debt_pos).into())
}

fn swap_or_net_collateral_to_debt(
    env: &Env,
    caller: &Address,
    collateral_token: &Address,
    debt_token: &Address,
    collateral_amount: i128,
    swap: &StrategySwap,
) -> i128 {
    if collateral_token == debt_token {
        return collateral_amount;
    }

    swap_tokens(
        env,
        caller,
        collateral_token,
        collateral_amount,
        debt_token,
        swap,
    )
}

fn close_remaining_collateral_if_requested(
    env: &Env,
    account: &mut Account,
    caller: &Address,
    cache: &mut Cache,
    close_position: bool,
) {
    if !close_position {
        return;
    }

    assert_with_error!(
        env,
        account.borrow_positions.is_empty(),
        CollateralError::CannotCloseWithRemainingDebt
    );

    execute_withdraw_all(env, account, caller, cache);
}
