//! "Multiply" (levered long) strategy.
//!
//! Borrows and supplies through an aggregator route in one transaction.

use crate::events::InitialMultiplyPaymentEvent;
use common::errors::{CollateralError, GenericError, StrategyError};
use controller_interface::types::{HubAssetKey, PositionMode, StrategySwap};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::helpers;
use crate::strategies::{
    open_strategy_borrow, prefetch_strategy_oracles, strategy_finalize, swap_tokens,
};
use crate::{positions::supply, storage, validation, Controller, ControllerArgs, ControllerClient};

/// Parameters for `process_multiply`.
pub struct MultiplyParams<'a> {
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

pub fn process_multiply(env: &Env, caller: &Address, params: MultiplyParams<'_>) -> u64 {
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

    // The swap leg needs distinct underlying tokens; the same token on two hubs
    // cannot be levered against itself.
    assert_with_error!(
        env,
        collateral.asset != debt.asset,
        GenericError::AssetsAreTheSame
    );

    // Allow-list accepted modes so only supported account modes reach multiply.
    assert_with_error!(
        env,
        matches!(
            mode,
            PositionMode::Multiply | PositionMode::Long | PositionMode::Short
        ),
        CollateralError::InvalidPositionMode
    );

    validation::require_positive_amount(env, debt_to_flash_loan);

    let (collateral_amount, debt_extra) = collect_initial_multiply_payment(
        env,
        caller,
        collateral,
        debt,
        &initial_payment,
        &convert_swap,
    );

    // Strategy borrows are risk-increasing.
    let mut cache = Cache::new(env);

    let collateral_config = cache.cached_asset_config(&collateral.asset);
    assert_with_error!(
        env,
        collateral_config.can_supply(),
        CollateralError::NotCollateral
    );

    let (account_id, mut account) = helpers::load_or_create_account(
        env,
        caller,
        account_id,
        spoke_id,
        mode,
        helpers::AccountGuard::Multiply,
        &mut cache,
    );

    let extra_assets =
        soroban_sdk::vec![env, collateral.asset.clone(), debt.asset.clone()];
    prefetch_strategy_oracles(&mut cache, &account, &extra_assets);

    // D{debt_token.decimals}{Token(debt_token)} net borrow received after protocol fee
    // on `debt`'s hub market.
    let amount_received =
        open_strategy_borrow(env, &mut cache, &mut account, debt, debt_to_flash_loan);

    // D{debt_token.decimals}{Token(debt_token)} net borrow plus same-token extra payment.
    let swap_amount_in = amount_received
        .checked_add(debt_extra)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    // D{debt_token.decimals}{Token(debt_token)} -> D{collateral_token.decimals}{Token(collateral_token)}.
    let swapped_collateral = swap_tokens(
        env,
        caller,
        &debt.asset,
        swap_amount_in,
        &collateral.asset,
        swap,
    );

    // D{collateral_token.decimals}{Token(collateral_token)} direct plus swapped collateral.
    let total_collateral = collateral_amount
        .checked_add(swapped_collateral)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    let deposit_assets = soroban_sdk::vec![env, (collateral.clone(), total_collateral)];

    supply::process_deposit(
        env,
        &env.current_contract_address(),
        &mut account,
        &deposit_assets,
        &mut cache,
    );

    strategy_finalize(env, account_id, &mut account, &mut cache);

    emit_multiply_initial_payment(env, &mut cache, account_id, initial_payment);

    account_id
}

fn collect_initial_multiply_payment(
    env: &Env,
    caller: &Address,
    collateral: &HubAssetKey,
    debt: &HubAssetKey,
    initial_payment: &Option<(HubAssetKey, i128)>,
    convert_swap: &Option<StrategySwap>,
) -> (i128, i128) {
    let mut collateral_amount = 0;
    let mut debt_extra = 0;

    if let Some((payment, payment_amount)) = initial_payment.as_ref() {
        validation::require_positive_amount(env, *payment_amount);

        // Only listed assets may invoke token contracts; payment asset is the
        // user-supplied call target, gated on its own hub coordinate.
        assert_with_error!(
            env,
            storage::get_spoke_asset(env, 0, payment).is_some(),
            GenericError::AssetNotSupported
        );

        let payment_tok = soroban_sdk::token::Client::new(env, &payment.asset);
        payment_tok.transfer(caller, env.current_contract_address(), payment_amount);

        if payment.asset == collateral.asset {
            collateral_amount = *payment_amount;
        } else if payment.asset == debt.asset {
            debt_extra = *payment_amount;
        } else {
            let convert = match convert_swap.as_ref() {
                Some(s) => s,
                None => panic_with_error!(env, StrategyError::ConvertStepsRequired),
            };
            // D{payment_token.decimals}{Token(payment_token)} -> Token(collateral_token).
            collateral_amount = swap_tokens(
                env,
                caller,
                &payment.asset,
                *payment_amount,
                &collateral.asset,
                convert,
            );
        }
    }

    (collateral_amount, debt_extra)
}

fn emit_multiply_initial_payment(
    env: &Env,
    cache: &mut Cache,
    account_id: u64,
    initial_payment: Option<(HubAssetKey, i128)>,
) {
    if let Some((payment, payment_amount)) = initial_payment {
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
