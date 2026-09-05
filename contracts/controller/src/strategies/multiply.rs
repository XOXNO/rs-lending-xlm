use crate::account;
use crate::events::InitialMultiplyPaymentEvent;
use common::errors::{CollateralError, GenericError, StrategyError};
use common::types::{HubAssetKey, PositionMode, StrategySwap};
use common::validation::require_positive_amount;
use soroban_sdk::{assert_with_error, panic_with_error, vec, Address, Env};

use crate::context::Context;
use crate::events::PositionAction;
use crate::payments::transfer_amount_measured;
use crate::positions::require_can_supply;
use crate::positions::supply;
use crate::risk::validation::require_authorized_caller;
use crate::strategies::{
    borrow_into_controller, prefetch_strategy_prices, strategy_finalize, swap_tokens,
    swap_tokens_or_passthrough,
};

pub(crate) struct MultiplyParams<'a> {
    pub account_id: u64,
    pub spoke_id: u32,
    pub collateral: &'a HubAssetKey,
    pub debt_to_flash_loan: i128,
    pub debt: &'a HubAssetKey,
    pub mode: PositionMode,
    pub swap: &'a StrategySwap,
    pub initial_payment: Option<(HubAssetKey, i128)>,
    pub convert_swap: Option<StrategySwap>,
}

/// Borrows and swaps into collateral to open or extend a leveraged position.
/// Includes optional initial funds and returns the account id after risk checks.
pub(crate) fn process_multiply(env: &Env, caller: &Address, params: MultiplyParams<'_>) -> u64 {
    require_authorized_caller(env, caller);

    let MultiplyParams {
        account_id,
        spoke_id,
        collateral,
        debt_to_flash_loan,
        debt,
        mode,
        swap,
        initial_payment,
        convert_swap,
    } = params;

    validate_multiply_request(env, collateral, debt, mode, debt_to_flash_loan);

    let mut cache = Context::new(env);
    let (account_id, mut account) = account::load_or_create_account(
        env,
        caller,
        account_id,
        spoke_id,
        mode,
        account::AccountGuard::Multiply,
        &mut cache,
    );
    require_can_supply(env, &mut cache, account.spoke_id, collateral);
    let mut extra_assets = vec![env, collateral.asset.clone(), debt.asset.clone()];
    if let Some((payment, _)) = initial_payment.as_ref() {
        extra_assets.push_back(payment.asset.clone());
    }
    prefetch_strategy_prices(&mut cache, &account, &extra_assets);

    let (collateral_amount, debt_extra) = collect_initial_multiply_payment(
        env,
        caller,
        collateral,
        debt,
        &initial_payment,
        &convert_swap,
    );

    let amount_received = borrow_into_controller(
        env,
        &mut account,
        debt,
        debt_to_flash_loan,
        true,
        PositionAction::Multiply,
        &mut cache,
    );

    let swap_amount_in = amount_received
        .checked_add(debt_extra)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    let swapped_collateral = swap_tokens_or_passthrough(
        env,
        caller,
        &debt.asset,
        swap_amount_in,
        &collateral.asset,
        swap,
    );

    let total_collateral = collateral_amount
        .checked_add(swapped_collateral)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    let deposit_assets = vec![env, (collateral.clone(), total_collateral)];
    supply::process_deposit(
        env,
        &env.current_contract_address(),
        &mut account,
        &deposit_assets,
        &mut cache,
    );

    strategy_finalize(env, account_id, &mut account, &mut cache);

    // Publish the optional requested payment only after account finalization.
    if let Some((payment, payment_amount)) = initial_payment {
        InitialMultiplyPaymentEvent {
            token: payment.asset,
            amount: payment_amount,
            account_id,
        }
        .publish(env);
    }

    account_id
}

/// Requires positive debt and distinct markets for Multiply, distinct assets
/// for Long/Short. Rejects other modes.
fn validate_multiply_request(
    env: &Env,
    collateral: &HubAssetKey,
    debt: &HubAssetKey,
    mode: PositionMode,
    debt_to_flash_loan: i128,
) {
    match mode {
        PositionMode::Multiply => {
            assert_with_error!(env, collateral != debt, GenericError::AssetsAreTheSame);
        }

        PositionMode::Long | PositionMode::Short => {
            assert_with_error!(
                env,
                collateral.asset != debt.asset,
                GenericError::AssetsAreTheSame
            );
        }
        _ => panic_with_error!(env, CollateralError::InvalidPositionMode),
    }
    require_positive_amount(env, debt_to_flash_loan);
}

/// Collects measured initial funds as `(collateral, debt)`, or zeros if absent.
/// Third-asset payments require a conversion route into collateral.
fn collect_initial_multiply_payment(
    env: &Env,
    caller: &Address,
    collateral: &HubAssetKey,
    debt: &HubAssetKey,
    initial_payment: &Option<(HubAssetKey, i128)>,
    convert_swap: &Option<StrategySwap>,
) -> (i128, i128) {
    let Some((payment, payment_amount)) = initial_payment.as_ref() else {
        return (0, 0);
    };

    require_positive_amount(env, *payment_amount);

    let received = transfer_amount_measured(
        env,
        &payment.asset,
        caller,
        &env.current_contract_address(),
        *payment_amount,
        GenericError::AmountMustBePositive,
    );

    if payment.asset == collateral.asset {
        (received, 0)
    } else if payment.asset == debt.asset {
        (0, received)
    } else {
        let Some(convert) = convert_swap.as_ref() else {
            panic_with_error!(env, StrategyError::ConvertStepsRequired);
        };

        let collateral_amount = swap_tokens(
            env,
            caller,
            &payment.asset,
            received,
            &collateral.asset,
            convert,
        );
        (collateral_amount, 0)
    }
}
