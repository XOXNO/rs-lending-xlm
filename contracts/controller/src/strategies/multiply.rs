//! Multiply strategy: opens or extends a leveraged position by borrowing a
//! debt asset, swapping it into the collateral asset, and depositing the
//! result together with any optional up-front payment.

use crate::account;
use crate::events::InitialMultiplyPaymentEvent;
use common::errors::{CollateralError, GenericError, StrategyError};
use common::types::{Account, HubAssetKey, PositionMode, StrategySwap};
use common::validation::require_positive_amount;
use soroban_sdk::{assert_with_error, panic_with_error, token, vec, Address, Env};

use crate::context::Cache;
use crate::positions::require_can_supply;
use crate::strategies::{
    borrow_for_strategy, prefetch_strategy_prices, strategy_finalize, swap_tokens,
    swap_tokens_or_passthrough,
};
use crate::{positions::supply, risk::validation};

/// Inputs to [`process_multiply`]: the target account and spoke, the
/// collateral and debt hub assets, the amount of debt to borrow, the position
/// mode, the swap route from debt to collateral, and an optional up-front
/// payment (with its own conversion route): the debt-side portion is added to
/// the borrowed amount before the swap, while the collateral-side portion is
/// combined with the swap output before deposit.
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

/// Opens or extends a multiply position for `caller`. Requires `caller`
/// authorization, rejects nested flash loans, and validates the collateral/debt
/// pair against `mode`. For existing accounts requires owner or active
/// position-manager delegate (new accounts take `caller` as owner). Collects any
/// optional initial payment, borrows `debt_to_flash_loan` units of debt via the
/// pool strategy-borrow path (not a flash loan), swaps into collateral, deposits
/// the total, finalizes the account, and emits an `InitialMultiplyPaymentEvent`
/// if an initial payment was supplied. Returns the account id. Panics with
/// `MathOverflow` if combining the borrowed and paid-in amounts overflows.
pub(crate) fn process_multiply(env: &Env, caller: &Address, params: MultiplyParams<'_>) -> u64 {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

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

    let (account_id, mut account, mut cache) =
        prepare_multiply_account(env, caller, account_id, spoke_id, mode, collateral, debt);

    let (collateral_amount, debt_extra) = collect_initial_multiply_payment(
        env,
        caller,
        collateral,
        debt,
        &initial_payment,
        &convert_swap,
    );

    let amount_received =
        borrow_for_strategy(env, &mut account, debt, debt_to_flash_loan, &mut cache);

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

/// Loads or creates the target account under
/// `account::AccountGuard::Multiply`, requires that `collateral` can be
/// supplied on the account's spoke, and prefetches oracle prices for the
/// account plus the collateral and debt assets. Returns the resolved account
/// id, the account, and a populated `Cache`.
fn prepare_multiply_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    spoke_id: u32,
    mode: PositionMode,
    collateral: &HubAssetKey,
    debt: &HubAssetKey,
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
    let extra_assets = vec![env, collateral.asset.clone(), debt.asset.clone()];
    prefetch_strategy_prices(&mut cache, &account, &extra_assets);
    (account_id, account, cache)
}

/// Validates the collateral/debt pair for `mode`: for `Multiply`, requires
/// the two hub asset keys to differ; for `Long` or `Short`, requires only the
/// underlying asset addresses to differ. Panics with
/// `CollateralError::InvalidPositionMode` for any other mode, and requires
/// `debt_to_flash_loan` to be positive.
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

/// Returns `(0, 0)` if `initial_payment` is `None`. Otherwise transfers the
/// payment amount from `caller` to the controller and requires it to be
/// positive. If the payment asset matches `collateral`, returns it as the
/// collateral-side amount; if it matches `debt`, returns it as the debt-side
/// amount; otherwise swaps it into collateral using `convert_swap`, panicking
/// with `StrategyError::ConvertStepsRequired` if `convert_swap` is `None`.
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

    let payment_tok = token::Client::new(env, &payment.asset);
    payment_tok.transfer(caller, env.current_contract_address(), payment_amount);

    if payment.asset == collateral.asset {
        (*payment_amount, 0)
    } else if payment.asset == debt.asset {
        (0, *payment_amount)
    } else {
        let Some(convert) = convert_swap.as_ref() else {
            panic_with_error!(env, StrategyError::ConvertStepsRequired);
        };

        let collateral_amount = swap_tokens(
            env,
            caller,
            &payment.asset,
            *payment_amount,
            &collateral.asset,
            convert,
        );
        (collateral_amount, 0)
    }
}

/// Publishes an `InitialMultiplyPaymentEvent` for `account_id` if
/// `initial_payment` is `Some`. Does nothing otherwise.
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
