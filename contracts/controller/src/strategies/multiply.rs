//! Leveraged multiply strategy: opens a collateral/debt position via a
//! flash-loan-funded swap, for `Multiply`, `Long`, or `Short` modes.

use crate::account;
use crate::events::InitialMultiplyPaymentEvent;
use common::errors::{CollateralError, GenericError, StrategyError};
use common::types::{Account, HubAssetKey, PositionMode, StrategySwap};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, token, vec, Address, Bytes, Env,
};
use stellar_macros::when_not_paused;

use crate::context::Cache;
use crate::spoke;
use crate::strategies::{
    borrow_for_strategy, prefetch_strategy_prices, strategy_finalize, swap_tokens,
    swap_tokens_or_passthrough,
};
use crate::{positions::supply, risk::validation, Controller, ControllerArgs, ControllerClient};

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

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn multiply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        collateral: HubAssetKey,
        debt_to_flash_loan: i128,
        debt: HubAssetKey,
        mode: PositionMode,
        swap: Bytes,
        initial_payment: Option<(HubAssetKey, i128)>,
        convert_swap: Option<Bytes>,
    ) -> u64 {
        process_multiply(
            &env,
            &caller,
            MultiplyParams {
                account_id,
                spoke_id,
                collateral: &collateral,
                debt_to_flash_loan,
                debt: &debt,
                mode,
                swap: &swap,
                initial_payment,
                convert_swap,
            },
        )
    }
}

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
        &mut cache,
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

    // Passthrough if same asset (cross-hub rate arb only).
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

    strategy_finalize(env, account_id, &account, &mut cache);

    emit_multiply_initial_payment(env, &mut cache, account_id, initial_payment);

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
    let collateral_config =
        spoke::require_listed_active_config(env, &mut cache, account.spoke_id, collateral);
    assert_with_error!(
        env,
        collateral_config.can_supply(),
        CollateralError::NotCollateral
    );
    let extra_assets = vec![env, collateral.asset.clone(), debt.asset.clone()];
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
        // Reject identical (hub, asset); cross-hub same asset is rate arb.
        PositionMode::Multiply => {
            assert_with_error!(env, collateral != debt, GenericError::AssetsAreTheSame);
        }
        // Long/Short need distinct assets.
        PositionMode::Long | PositionMode::Short => {
            assert_with_error!(
                env,
                collateral.asset != debt.asset,
                GenericError::AssetsAreTheSame
            );
        }
        _ => panic_with_error!(env, CollateralError::InvalidPositionMode),
    }
    validation::require_positive_amount(env, debt_to_flash_loan);
}

/// Pulls the optional initial payment and returns its (collateral amount, same-token debt extra) contribution.
fn collect_initial_multiply_payment(
    env: &Env,
    caller: &Address,
    cache: &mut Cache,
    collateral: &HubAssetKey,
    debt: &HubAssetKey,
    initial_payment: &Option<(HubAssetKey, i128)>,
    convert_swap: &Option<StrategySwap>,
) -> (i128, i128) {
    let Some((payment, payment_amount)) = initial_payment.as_ref() else {
        return (0, 0);
    };

    validation::require_positive_amount(env, *payment_amount);

    // Fail fast on an unsupported/unpriceable payment token BEFORE invoking
    // its transfer — the aggregator reverts `OracleNotConfigured` for assets
    // outside the protocol. Also warms the price the payment event reads.
    cache.fetch_prices(&soroban_sdk::vec![env, payment.asset.clone()]);

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
        // D{payment_token.decimals}{Token(payment_token)} -> Token(collateral_token).
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

/// Publishes the initial-payment event with its USD value when a payment was made.
fn emit_multiply_initial_payment(
    env: &Env,
    cache: &mut Cache,
    account_id: u64,
    initial_payment: Option<(HubAssetKey, i128)>,
) {
    if let Some((payment, payment_amount)) = initial_payment {
        // A converted third-token payment is never a position asset, so it is
        // absent from the tx-local price map the position legs populate. Fetch
        // it so the cached read below resolves the event's USD value.
        cache.fetch_prices(&vec![env, payment.asset.clone()]);
        let feed = cache.cached_price(&payment.asset);
        let usd_value_wad = feed.usd_value_wad(env, payment_amount).raw();
        InitialMultiplyPaymentEvent {
            token: payment.asset,
            amount: payment_amount,
            usd_value_wad,
            account_id,
        }
        .publish(env);
    }
}
