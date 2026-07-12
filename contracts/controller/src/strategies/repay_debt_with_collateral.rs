//! Repays debt by withdrawing and swapping collateral.

use common::errors::{CollateralError, GenericError};
use common::types::{Account, HubAssetKey, StrategySwap};
use soroban_sdk::{assert_with_error, contractimpl, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::account;
use crate::context::Cache;
use crate::events;
use crate::positions::{get_debt_position_or_panic, get_supply_position_or_panic};
use crate::strategies::{
    execute_withdraw_all, net_settle_collateral_against_debt, prefetch_strategy_oracles,
    repay_debt_from_controller, strategy_finalize, swap_tokens_or_passthrough,
    withdraw_collateral_to_controller, StrategyRepay, StrategyWithdraw,
};
use crate::{risk::validation, storage, Controller, ControllerArgs, ControllerClient};

pub struct RepayWithCollateralParams<'a> {
    pub account_id: u64,
    pub collateral: &'a HubAssetKey,
    pub collateral_amount: i128,
    pub debt: &'a HubAssetKey,
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
        collateral: HubAssetKey,
        collateral_amount: i128,
        debt: HubAssetKey,
        swap: Bytes,
        close_position: bool,
    ) {
        process_repay_debt_with_collateral(
            &env,
            &caller,
            RepayWithCollateralParams {
                account_id,
                collateral: &collateral,
                collateral_amount,
                debt: &debt,
                swap: &swap,
                close_position,
            },
        );
    }
}

/// Withdraws collateral, swaps it to the debt token, and repays the debt position.
pub fn process_repay_debt_with_collateral(
    env: &Env,
    caller: &Address,
    params: RepayWithCollateralParams<'_>,
) {
    let RepayWithCollateralParams {
        account_id,
        collateral,
        collateral_amount,
        debt,
        swap,
        close_position,
    } = params;

    caller.require_auth();
    validation::require_not_flash_loaning(env);
    validation::require_positive_amount(env, collateral_amount);

    // The withdraw and repay legs settle on their own hubs; neither path gates
    // hub membership, so assert it here.
    validation::require_hub_active(env, collateral.hub_id);
    validation::require_hub_active(env, debt.hub_id);

    let mut account = storage::get_account(env, account_id);
    account::require_owner_or_delegate(env, account_id, caller, &account.owner);

    let mut cache = Cache::new(env);

    let extra_assets = soroban_sdk::vec![env, collateral.asset.clone(), debt.asset.clone()];
    prefetch_strategy_oracles(&mut cache, &account, &extra_assets);

    if collateral == debt {
        // Identical hub-asset: net the two legs in the pool with zero token
        // transfer instead of withdrawing then immediately repaying the same
        // real amount back in. Strictly more available than the transfer
        // path below — it never needs idle pool liquidity, only that both
        // positions exist.
        assert_with_error!(env, swap.is_empty(), GenericError::InvalidPayments);
        net_settle_collateral_against_debt(
            env,
            &mut account,
            &mut cache,
            collateral,
            collateral_amount,
            events::PositionAction::RpColNet,
        );
    } else {
        let collateral_pos = get_supply_position_or_panic(env, &account, collateral);
        let debt_pos = get_debt_position_or_panic(env, &account, debt);

        // D{collateral_token.decimals}{Token(collateral_token)} requested withdrawal to live balance delta.
        let actual_withdrawn = withdraw_collateral_to_controller(
            env,
            &mut account,
            &mut cache,
            StrategyWithdraw {
                hub_asset: collateral,
                amount: collateral_amount,
                position: &collateral_pos,
                action: events::PositionAction::RpColWd,
            },
        );

        // D{collateral_token.decimals}{Token(collateral_token)} -> Token(debt_token), unless same asset.
        let debt_available = swap_tokens_or_passthrough(
            env,
            caller,
            &collateral.asset,
            actual_withdrawn,
            &debt.asset,
            swap,
        );
        repay_debt_from_controller(
            env,
            &mut account,
            &mut cache,
            caller,
            StrategyRepay {
                debt,
                debt_available,
                debt_pos: &debt_pos,
                action: events::PositionAction::RpColR,
            },
        );
    }

    close_remaining_collateral_if_requested(env, &mut account, caller, &mut cache, close_position);

    strategy_finalize(env, account_id, &account, &mut cache);
}

/// Withdraws all remaining collateral when closing, requiring no debt remains.
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
