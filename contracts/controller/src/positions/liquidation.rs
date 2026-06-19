//! Liquidation and keeper bad-debt cleanup.
//!
//! Pipeline: auth → aggregate → cache → validate inputs → plan → apply repay
//! → apply seize → post-checks → persist → emit. Liquidation requires health
//! factor below one, prices with `OraclePolicy::Liquidation`, repays debt,
//! seizes collateral, and refunds payment above the close amount. Bad-debt
//! cleanup socializes residual debt only when collateral is below the USD threshold.

use crate::events::CleanBadDebtEvent;
use common::errors::{CollateralError, GenericError};
use common::math::fp::Wad;
use controller_interface::types::{
    Account, AccountPosition, AccountPositionType, DebtPosition, LiquidationResult, Payment,
    PoolAction, PoolWithdrawEntry, RepayEntry, ScaledPositionRaw, SeizeEntry,
};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Env, Vec};
use stellar_macros::only_role;

use super::liquidation_math::*;
use super::{persist_account_positions, repay, withdraw, AggregatedPayments, PositionSides};
use crate::cache::Cache;
use crate::events;
use crate::external::pool::pool_seize_position_call;
use crate::external::sac::sac_transfer_call;
use crate::oracle::policy::OraclePolicy;
use crate::positions::make_pool_action;
use crate::storage::{iter_debt_positions, iter_typed_positions};
use crate::{
    helpers::{self, utils},
    storage, validation, Controller, ControllerArgs, ControllerClient,
};

#[contractimpl]
impl Controller {
    pub fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(Address, i128)>,
    ) {
        process_liquidation(&env, &liquidator, account_id, &debt_payments);
    }

    #[only_role(caller, "KEEPER")]
    pub fn clean_bad_debt(env: Env, caller: Address, account_id: u64) {
        validation::require_not_flash_loaning(&env);

        clean_bad_debt_standalone(&env, account_id);
    }
}

/// Liquidates an underwater account using protocol prices, bonus math, and pool calls.
pub fn process_liquidation(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    debt_payments: &Vec<Payment>,
) {
    liquidator.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);

    let aggregated = utils::aggregate_positive_payments(env, debt_payments);

    // Liquidation denies stale, deviation, and TWAP loosening. Outside the last
    // tolerance band, `UnsafePriceNotAllowed` prevents seizure at a single-source
    // price; inside the bands the standard primary/midpoint selection applies.
    let mut cache = Cache::new(env, OraclePolicy::Liquidation);

    validate_liquidation_inputs(env, &account, liquidator, &aggregated, &mut cache);

    let liquidation_plan = build_liquidation_plan(env, &account, &aggregated, &mut cache);
    let result = liquidation_plan.into_result();

    validation::require_non_empty_payments(env, &result.repaid);

    apply_liquidation_repayments(env, liquidator, &mut account, &result.repaid, &mut cache);
    apply_liquidation_seizures(env, liquidator, &mut account, &result.seized, &mut cache);

    let post_totals = helpers::calculate_account_risk_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    let will_socialize =
        is_socializable_bad_debt(post_totals.total_debt, post_totals.total_collateral);

    persist_account_positions(env, account_id, &account, PositionSides::BOTH, false);

    // Reuse the post-liquidation account snapshot for bad-debt cleanup.
    check_bad_debt_after_liquidation(
        env,
        &mut cache,
        account_id,
        &account,
        post_totals.total_collateral,
        post_totals.total_debt,
        will_socialize,
    );
    cache.emit_position_batch(account_id, &account);
}

fn validate_liquidation_inputs(
    env: &Env,
    account: &Account,
    liquidator: &Address,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    validation::require_non_empty_payments(env, aggregated);

    assert_with_error!(
        env,
        account.owner != *liquidator,
        GenericError::AccountNotInMarket
    );

    for (asset, _) in aggregated.iter() {
        validation::require_asset_supported(env, cache, &asset);
    }
}

/// Computes the liquidation outcome (repayments, seizures, refunds) from the
/// account snapshot and the liquidator's aggregated debt payments; mutates nothing.
pub(crate) fn execute_liquidation(
    env: &Env,
    account: &Account,
    aggregated_debt: &Vec<Payment>,
    cache: &mut Cache,
) -> LiquidationResult {
    build_liquidation_plan(env, account, aggregated_debt, cache).into_result()
}

fn build_liquidation_plan(
    env: &Env,
    account: &Account,
    aggregated_debt: &Vec<Payment>,
    cache: &mut Cache,
) -> LiquidationPlan {
    // One totals pass feeds both the HF gate and the snapshot. A debt-free
    // account carries a saturated health factor that fails the `hf < ONE` gate,
    // but the early panic skips pricing it.
    if account.borrow_positions.is_empty() {
        panic_with_error!(env, CollateralError::HealthFactorTooHigh);
    }
    let totals = helpers::calculate_account_risk_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    assert_with_error!(
        env,
        totals.health_factor < Wad::ONE,
        CollateralError::HealthFactorTooHigh
    );

    let (proportion_seized, bonus_bounds) = calculate_seizure_proportions(
        env,
        account,
        totals.total_collateral,
        totals.weighted_collateral,
        cache,
    );

    let snap = LiquidationSnapshot {
        total_debt: totals.total_debt,
        total_collateral: totals.total_collateral,
        weighted_coll: totals.weighted_collateral,
        proportion_seized,
        hf: totals.health_factor,
    };

    let repayment =
        normalize_repayment_plan(env, account, aggregated_debt, &snap, bonus_bounds, cache);

    let seized_collaterals =
        calculate_seized_collateral(env, account, totals.total_collateral, &repayment, cache);

    let plan = LiquidationPlan {
        repayment,
        seized: seized_collaterals,
    };
    plan.validate(env);
    plan
}

fn apply_liquidation_repayments(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    repaid: &Vec<RepayEntry>,
    cache: &mut Cache,
) {
    // Transfer each repayment in while building the actions for one bulk pool call.
    let pool_addr = cache.cached_pool_address();
    let mut actions: Vec<PoolAction> = Vec::new(env);
    for entry in repaid.iter() {
        // All SAC transfers go through the wrapper so the harness can replace it.
        sac_transfer_call(env, &entry.asset, liquidator, &pool_addr, &entry.amount);

        let position: DebtPosition =
            (&validation::expect_invariant(env, account.borrow_positions.get(entry.asset.clone())))
                .into();
        actions.push_back(make_pool_action(
            &position,
            entry.amount,
            entry.asset.clone(),
        ));
    }
    repay::settle_repay_actions(
        env,
        account,
        liquidator,
        events::PositionAction::LiqRepay,
        &actions,
        cache,
    );
}

fn apply_liquidation_seizures(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut Cache,
) {
    // Build all seizure entries for one bulk pool call.
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    for entry in seized.iter() {
        let position: AccountPosition =
            (&validation::expect_invariant(env, account.supply_positions.get(entry.asset.clone())))
                .into();
        entries.push_back(PoolWithdrawEntry {
            action: make_pool_action(&position, entry.amount, entry.asset.clone()),
            protocol_fee: entry.protocol_fee,
        });
    }
    withdraw::settle_withdraw_entries(
        env,
        account,
        liquidator,
        true,
        events::PositionAction::LiqSeize,
        &entries,
        cache,
    );
}

fn check_bad_debt_after_liquidation(
    env: &Env,
    cache: &mut Cache,
    account_id: u64,
    account: &Account,
    total_collateral_usd: Wad,
    total_debt_usd: Wad,
    will_socialize: bool,
) {
    if account.borrow_positions.is_empty() {
        helpers::cleanup_account_if_empty(env, account, account_id);
        return;
    }

    if will_socialize {
        execute_bad_debt_cleanup(
            env,
            cache,
            account_id,
            account,
            total_debt_usd.raw(),
            total_collateral_usd.raw(),
        );
    }
}

/// Socializes small residual bad debt by seizing all collateral and debt shares.
pub fn clean_bad_debt_standalone(env: &Env, account_id: u64) {
    // Success removes the account; failure reverts atomically, so no keep-alive is needed.
    // Cleanup uses the same `Liquidation` policy as inline post-liquidation cleanup.
    let mut cache = Cache::new(env, OraclePolicy::Liquidation);
    let account = storage::get_account(env, account_id);

    assert_with_error!(
        env,
        !account.borrow_positions.is_empty(),
        CollateralError::PositionNotFound
    );

    let totals = helpers::calculate_account_risk_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    if !is_socializable_bad_debt(totals.total_debt, totals.total_collateral) {
        panic_with_error!(env, CollateralError::CannotCleanBadDebt);
    }

    execute_bad_debt_cleanup(
        env,
        &mut cache,
        account_id,
        &account,
        totals.total_debt.raw(),
        totals.total_collateral.raw(),
    );
}

fn execute_bad_debt_cleanup(
    env: &Env,
    cache: &mut Cache,
    account_id: u64,
    account: &Account,
    total_debt_usd: i128,
    total_collateral_usd: i128,
) {
    for (asset, position) in iter_typed_positions(&account.supply_positions) {
        seize_pool_position(
            env,
            cache,
            AccountPositionType::Deposit,
            &asset,
            (&position).into(),
        );
    }

    for (asset, position) in iter_debt_positions(&account.borrow_positions) {
        seize_pool_position(
            env,
            cache,
            AccountPositionType::Borrow,
            &asset,
            (&position).into(),
        );
    }

    CleanBadDebtEvent {
        account_id,
        total_borrow_usd_wad: total_debt_usd,
        total_collateral_usd_wad: total_collateral_usd,
    }
    .publish(env);

    helpers::remove_account(env, account_id);
}

fn seize_pool_position(
    env: &Env,
    cache: &mut Cache,
    side: AccountPositionType,
    asset: &Address,
    position: ScaledPositionRaw,
) {
    let pool_addr = cache.cached_pool_address();
    pool_seize_position_call(env, &pool_addr, asset, side, position);
}
