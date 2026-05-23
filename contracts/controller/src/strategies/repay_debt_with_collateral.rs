use common::errors::CollateralError;
use common::types::{Account, AccountPosition, AggregatorSwap};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env};
use stellar_macros::when_not_paused;

use crate::cache::ControllerCache;
use crate::oracle::policy::OraclePolicy;
use crate::strategies::helpers::{
    execute_withdraw_all, repay_debt_from_controller, strategy_finalize, swap_tokens,
    withdraw_collateral_to_controller, StrategyRepay, StrategyWithdraw,
};
use crate::{storage, validation, Controller, ControllerArgs, ControllerClient};

/// Parameters for `process_repay_debt_with_collateral`.
pub struct RepayWithCollateralParams<'a> {
    pub account_id: u64,
    pub collateral_token: &'a Address,
    pub collateral_amount: i128,
    pub debt_token: &'a Address,
    pub swap: &'a AggregatorSwap,
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
        swap: AggregatorSwap,
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
    validation::require_amount_positive(env, collateral_amount);
    // Skip the slippage-floor check for the same-asset short-circuit (no
    // swap occurs).
    if collateral_token != debt_token {
        validation::require_amount_positive(env, swap.total_min_out);
    }

    // The same-asset flow is intentionally allowed: self-collateralized
    // positions (e.g. stablecoin/stablecoin) can net the two legs atomically
    // without routing through the aggregator.

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);

    let mut cache = ControllerCache::new(env, OraclePolicy::RiskIncreasing);

    let (collateral_pos, debt_pos) =
        load_repay_with_collateral_positions(env, &account, collateral_token, debt_token);

    let actual_withdrawn = withdraw_collateral_to_controller(
        env,
        &mut account,
        &mut cache,
        caller,
        StrategyWithdraw {
            asset: collateral_token,
            amount: collateral_amount,
            position: &collateral_pos,
            action: symbol_short!("rp_col_wd"),
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
        &mut cache,
        caller,
        StrategyRepay {
            debt_token,
            debt_available,
            debt_pos: &debt_pos,
            action: symbol_short!("rp_col_r"),
        },
    );

    close_remaining_collateral_if_requested(
        env,
        &mut account,
        account_id,
        caller,
        &mut cache,
        close_position,
    );

    strategy_finalize(env, account_id, &mut account, &mut cache);
}

fn load_repay_with_collateral_positions(
    env: &Env,
    account: &Account,
    collateral_token: &Address,
    debt_token: &Address,
) -> (AccountPosition, AccountPosition) {
    // Validate both positions before moving any tokens so a missing
    // position surfaces as its specific error rather than a host panic on
    // a later transfer.
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
    swap: &AggregatorSwap,
) -> i128 {
    if collateral_token == debt_token {
        return collateral_amount;
    }

    swap_tokens(
        env,
        collateral_token,
        collateral_amount,
        debt_token,
        swap,
        caller,
    )
}

fn close_remaining_collateral_if_requested(
    env: &Env,
    account: &mut Account,
    account_id: u64,
    caller: &Address,
    cache: &mut ControllerCache,
    close_position: bool,
) {
    if !close_position {
        return;
    }

    if !account.borrow_positions.is_empty() {
        panic_with_error!(env, CollateralError::CannotCloseWithRemainingDebt);
    }

    execute_withdraw_all(env, account, account_id, caller, cache);
}
