
use crate::account;
use crate::events::InitialMultiplyPaymentEvent;
use common::errors::{CollateralError, GenericError, StrategyError};
use common::types::{Account, HubAssetKey, PositionMode, StrategySwap};
use common::validation::require_positive_amount;
use soroban_sdk::{assert_with_error, panic_with_error, vec, Address, Env};

use crate::context::Cache;
use crate::positions::require_can_supply;
use crate::events::PositionAction;
use crate::strategies::{
    borrow_into_controller, prefetch_strategy_prices, strategy_finalize, swap_tokens,
    swap_tokens_or_passthrough,
};
use crate::{positions::supply};

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

pub(crate) fn process_multiply(env: &Env, caller: &Address, params: MultiplyParams<'_>) -> u64 {
    crate::strategies::require_strategy_caller(env, caller);

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

    let (account_id, mut account, mut cache) = prepare_multiply_account(
        env,
        caller,
        account_id,
        spoke_id,
        mode,
        collateral,
        debt,
        &initial_payment,
    );

    let (collateral_amount, debt_extra) = collect_initial_multiply_payment(
        env,
        caller,
        collateral,
        debt,
        &initial_payment,
        &convert_swap,
    );

    let amount_received =
        borrow_into_controller(env, &mut account, debt, debt_to_flash_loan, true, PositionAction::Multiply, &mut cache);

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

    emit_multiply_initial_payment(env, account_id, initial_payment);

    account_id
}

fn prepare_multiply_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    spoke_id: u32,
    mode: PositionMode,
    collateral: &HubAssetKey,
    debt: &HubAssetKey,
    initial_payment: &Option<(HubAssetKey, i128)>,
) -> (u64, Account, Cache) {
    let mut cache = Cache::new(env);
    let (account_id, account) = account::load_or_create_account(
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
    (account_id, account, cache)
}

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

    let received = common::token::transfer_amount_measured(
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

fn emit_multiply_initial_payment(
    env: &Env,
    account_id: u64,
    initial_payment: Option<(HubAssetKey, i128)>,
) {
    if let Some((payment, payment_amount)) = initial_payment {
        InitialMultiplyPaymentEvent {
            token: payment.asset,
            amount: payment_amount,
            account_id,
        }
        .publish(env);
    }
}
