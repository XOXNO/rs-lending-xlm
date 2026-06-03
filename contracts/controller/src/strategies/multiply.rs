//! "Multiply" (levered long) strategy.
//!
//! Recursive borrow + supply in one transaction via the aggregator. Uses
//! `OraclePolicy::RiskIncreasing` (strictest pricing); all safety lives in
//! the parent module's helpers.

use common::errors::{CollateralError, GenericError, StrategyError};
use common::events::InitialMultiplyPaymentEvent;
use common::types::{Account, AssetConfig, PositionMode, StrategySwap};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::oracle::policy::OraclePolicy;
use crate::strategies::helpers::{open_strategy_borrow, strategy_finalize, swap_tokens};
use crate::{
    positions::supply, storage, utils, validation, Controller, ControllerArgs, ControllerClient,
};

/// Parameters for `process_multiply`. Mirrors the public entrypoint args.
pub struct MultiplyParams<'a> {
    pub account_id: u64,
    pub e_mode_category: u32,
    pub collateral_token: &'a Address,
    pub debt_to_flash_loan: i128,
    pub debt_token: &'a Address,
    pub mode: PositionMode,
    pub swap: &'a StrategySwap,
    pub initial_payment: Option<(Address, i128)>,
    pub convert_swap: Option<StrategySwap>,
}

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn multiply(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        collateral_token: Address,
        debt_to_flash_loan: i128,
        debt_token: Address,
        mode: PositionMode,
        swap: Bytes,
        initial_payment: Option<(Address, i128)>,
        convert_swap: Option<Bytes>,
    ) -> u64 {
        process_multiply(
            &env,
            &caller,
            MultiplyParams {
                account_id,
                e_mode_category,
                collateral_token: &collateral_token,
                debt_to_flash_loan,
                debt_token: &debt_token,
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
        e_mode_category,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        mode,
        swap,
        initial_payment,
        convert_swap,
    } = params;

    assert_with_error!(
        env,
        collateral_token != debt_token,
        GenericError::AssetsAreTheSame
    );

    // Allow-list rather than `!= Normal` so a future `PositionMode` variant
    // cannot silently slip through multiply.
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
        collateral_token,
        debt_token,
        &initial_payment,
        &convert_swap,
    );

    // Strategy borrows are risk-increasing.
    let mut cache = Cache::new(env, OraclePolicy::RiskIncreasing);

    let collateral_config = cache.cached_asset_config(collateral_token);
    assert_with_error!(
        env,
        collateral_config.can_supply(),
        CollateralError::NotCollateral
    );

    let (account_id, mut account) = load_or_create_multiply_account(
        env,
        caller,
        account_id,
        e_mode_category,
        collateral_token,
        &collateral_config,
        mode,
    );

    let amount_received = open_strategy_borrow(
        env,
        &mut cache,
        &mut account,
        debt_token,
        debt_to_flash_loan,
    );

    let swap_amount_in = amount_received
        .checked_add(debt_extra)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    let swapped_collateral = swap_tokens(env, debt_token, swap_amount_in, collateral_token, swap);

    let total_collateral = collateral_amount
        .checked_add(swapped_collateral)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    let deposit_assets = soroban_sdk::vec![env, (collateral_token.clone(), total_collateral)];

    supply::process_deposit(
        env,
        &env.current_contract_address(),
        &mut account,
        &deposit_assets,
        &mut cache,
    );

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

    emit_multiply_initial_payment(env, &mut cache, account_id, initial_payment);

    account_id
}

fn collect_initial_multiply_payment(
    env: &Env,
    caller: &Address,
    collateral_token: &Address,
    debt_token: &Address,
    initial_payment: &Option<(Address, i128)>,
    convert_swap: &Option<StrategySwap>,
) -> (i128, i128) {
    let mut collateral_amount = 0;
    let mut debt_extra = 0;

    if let Some((payment_token, payment_amount)) = initial_payment.as_ref() {
        validation::require_positive_amount(env, *payment_amount);

        let payment_tok = soroban_sdk::token::Client::new(env, payment_token);
        payment_tok.transfer(caller, env.current_contract_address(), payment_amount);

        if *payment_token == *collateral_token {
            collateral_amount = *payment_amount;
        } else if *payment_token == *debt_token {
            debt_extra = *payment_amount;
        } else {
            let convert = match convert_swap.as_ref() {
                Some(s) => s,
                None => panic_with_error!(env, StrategyError::ConvertStepsRequired),
            };
            collateral_amount = swap_tokens(
                env,
                payment_token,
                *payment_amount,
                collateral_token,
                convert,
            );
        }
    }

    (collateral_amount, debt_extra)
}

fn load_or_create_multiply_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    collateral_token: &Address,
    collateral_config: &AssetConfig,
    mode: PositionMode,
) -> (u64, Account) {
    if account_id == 0 {
        let is_isolated = collateral_config.is_isolated_asset;
        let isolated_asset = if is_isolated {
            Some(collateral_token.clone())
        } else {
            None
        };
        // `create_account` returns the in-memory snapshot it just wrote, so
        // there's no need to re-read all 3 keys from storage.
        return utils::create_account(
            env,
            caller,
            e_mode_category,
            mode,
            is_isolated,
            isolated_asset,
        );
    }

    let account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);
    assert_with_error!(env, account.mode == mode, GenericError::AccountModeMismatch);
    (account_id, account)
}

fn emit_multiply_initial_payment(
    env: &Env,
    cache: &mut Cache,
    account_id: u64,
    initial_payment: Option<(Address, i128)>,
) {
    if let Some((payment_token, payment_amount)) = initial_payment {
        let feed = cache.cached_price(&payment_token);
        let usd_value_wad = feed.usd_value_wad(env, payment_amount).raw();
        InitialMultiplyPaymentEvent {
            token: payment_token,
            amount: payment_amount,
            usd_value_wad,
            account_id,
        }
        .publish(env);
    }
}
