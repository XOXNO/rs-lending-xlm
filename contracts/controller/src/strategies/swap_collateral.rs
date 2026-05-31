use common::errors::{CollateralError, EModeError, FlashLoanError, GenericError};
use common::types::{Account, AccountPosition, AccountPositionType, AggregatorSwap};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, symbol_short, Address, Env};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::oracle::policy::OraclePolicy;
use crate::strategies::helpers::{
    strategy_finalize, swap_tokens, withdraw_collateral_to_controller, StrategyWithdraw,
};
use crate::{
    emode, positions::supply, storage, validation, Controller, ControllerArgs, ControllerClient,
};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn swap_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        current_collateral: Address,
        amount: i128,
        new_collateral: Address,
        swap: AggregatorSwap,
    ) {
        process_swap_collateral(
            &env,
            &caller,
            account_id,
            &current_collateral,
            amount,
            &new_collateral,
            &swap,
        );
    }
}

// Swaps collateral to different token.
pub fn process_swap_collateral(
    env: &Env,
    caller: &Address,
    account_id: u64,
    current_collateral: &Address,
    from_amount: i128,
    new_collateral: &Address,
    swap: &AggregatorSwap,
) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    assert_with_error!(
        env,
        current_collateral != new_collateral,
        GenericError::AssetsAreTheSame
    );

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);

    assert_with_error!(
        env,
        !account.is_isolated,
        FlashLoanError::SwapCollateralNoIso
    );

    // Debt-free swaps are risk-neutral: no borrow can be liquidated, so the
    // tightest oracle tolerance is unnecessary.
    let policy = if account.borrow_positions.is_empty() {
        OraclePolicy::RiskDecreasing
    } else {
        OraclePolicy::RiskIncreasing
    };
    let mut cache = Cache::new(env, policy);

    validation::require_amount_positive(env, from_amount);
    // Reject zero-floor swap requests at entry.
    validation::require_amount_positive(env, swap.total_min_out);

    validate_swap_new_collateral_preflight(env, &mut cache, &account, new_collateral);

    let current_pos: AccountPosition = (&account
        .supply_positions
        .get(current_collateral.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::CollateralPositionNotFound)))
        .into();

    let actual_withdrawn = withdraw_collateral_to_controller(
        env,
        &mut account,
        &mut cache,
        StrategyWithdraw {
            asset: current_collateral,
            amount: from_amount,
            position: &current_pos,
            action: symbol_short!("sw_col_wd"),
        },
    );

    let swapped_amount = swap_tokens(
        env,
        current_collateral,
        actual_withdrawn,
        new_collateral,
        swap,
    );

    let deposit_assets = soroban_sdk::vec![env, (new_collateral.clone(), swapped_amount)];
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
            supply_assets: &[current_collateral, new_collateral],
            borrow_assets: &[],
        },
    );
}

/// Rejects replacement collateral that cannot be supplied after the swap.
pub fn validate_swap_new_collateral_preflight(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    new_collateral: &Address,
) {
    // Reject deprecated e-mode categories before withdrawal/swap side effects:
    // the tx cannot deposit the replacement collateral anyway.
    let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);
    let config = emode::effective_asset_config(env, account, new_collateral, cache, &e_mode);
    if config.is_isolated_asset {
        // swap_collateral generally serves non-isolated positions only.
        // Isolated accounts use repayDebtWithCollateral to deleverage.
        panic_with_error!(env, EModeError::MixIsolatedCollateral);
    }
    emode::ensure_e_mode_compatible_with_asset(env, &config, account.e_mode_category_id);
    emode::validate_e_mode_asset(env, cache, account.e_mode_category_id, new_collateral);

    assert_with_error!(env, config.can_supply(), CollateralError::NotCollateral);

    // A new destination asset adds a position slot, so enforce deposit limits.
    if !account
        .supply_positions
        .contains_key(new_collateral.clone())
    {
        let new_assets = soroban_sdk::vec![env, (new_collateral.clone(), 0i128)];
        validation::validate_bulk_position_limits(
            env,
            account,
            AccountPositionType::Deposit,
            &new_assets,
        );
    }
}
