use common::constants::BAD_DEBT_USD_THRESHOLD;
use common::errors::{CollateralError, GenericError};
use common::events::{emit_clean_bad_debt, CleanBadDebtEvent};
use common::math::fp::Wad;
use common::types::{
    Account, AccountPosition, AccountPositionType, DebtPosition, LiquidationResult, Payment,
    RepayEntry, ScaledPositionRaw, SeizeEntry,
};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Symbol, Vec};
use stellar_macros::{only_role, when_not_paused};

use super::dust::{require_no_borrow_dust_for_assets, require_no_supply_dust_for_assets};
use super::liquidation_math::*;
use super::repay::RepaymentRequest;
use super::withdraw::{WithdrawFlags, WithdrawalRequest};
use super::{repay, withdraw, EventContext};
use crate::cache::ControllerCache;
use crate::cross_contract::pool::pool_seize_position_call;
use crate::oracle::policy::OraclePolicy;
use crate::positions;
use crate::storage::{iter_debt_positions, iter_typed_positions};
use crate::{helpers, storage, utils, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(Address, i128)>,
    ) {
        process_liquidation(&env, &liquidator, account_id, &debt_payments);
    }

    #[when_not_paused]
    #[only_role(caller, "KEEPER")]
    pub fn clean_bad_debt(env: Env, caller: Address, account_id: u64) {
        validation::require_not_flash_loaning(&env);

        clean_bad_debt_standalone(&env, account_id);
    }
}

// Executes liquidation.
pub fn process_liquidation(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    debt_payments: &Vec<Payment>,
) {
    // Stage 1: Pipelined Context Check
    liquidator.require_auth();
    validation::require_not_flash_loaning(env);
    validation::require_non_empty_payments(env, debt_payments);

    // Reject self-liquidation.
    let account_meta = storage::get_account_meta(env, account_id);
    if account_meta.owner == *liquidator {
        panic_with_error!(env, GenericError::AccountNotInMarket);
    }

    // Stage 2: State Resolution
    let debt_payment_plan = utils::aggregate_positive_payments(env, debt_payments);

    // Liquidation reduces protocol exposure.
    let mut cache = ControllerCache::new(env, OraclePolicy::Liquidation);

    for (asset, _) in debt_payment_plan.iter() {
        validation::require_asset_supported(env, &mut cache, &asset);
    }

    let mut account = storage::get_account(env, account_id);

    // Stage 3 & 4: Pre-flight Validation & Core Pool Execution
    // Calculate seizure and repayment.
    let result = execute_liquidation(env, &account, &debt_payment_plan, &mut cache);

    validation::require_non_empty_payments(env, &result.repaid);

    apply_liquidation_repayments(env, liquidator, &mut account, &result.repaid, &mut cache);
    apply_liquidation_seizures(env, liquidator, &mut account, &result.seized, &mut cache);

    // Stage 5: Post-flight Risk Gates
    // Per-leg dust gate. Scoped to the assets the liquidation actually
    // touched (seized supply + repaid debt). Other positions on the
    // account that drifted under floor due to price moves are not the
    // liquidation's concern and must not block the call.
    let (post_total_coll, post_total_debt, _) = helpers::calculate_account_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    let bad_debt_threshold = Wad::from_raw(BAD_DEBT_USD_THRESHOLD);
    let will_socialize = post_total_debt > post_total_coll && post_total_coll <= bad_debt_threshold;
    if !will_socialize {
        let seized_assets = seize_entry_assets(env, &result.seized);
        let repaid_assets = repay_entry_assets(env, &result.repaid);
        require_no_supply_dust_for_assets(env, &mut cache, &account, &seized_assets);
        require_no_borrow_dust_for_assets(env, &mut cache, &account, &repaid_assets);
    }

    // Stage 6: State Persistence
    // Persist position updates.
    storage::set_supply_positions(env, account_id, &account.supply_positions);
    storage::set_debt_positions(env, account_id, &account.borrow_positions);

    // Reuse the post-liquidation account snapshot for bad-debt cleanup.
    check_bad_debt_after_liquidation(env, &mut cache, account_id, &account);

    cache.flush_isolated_debts();
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();
}

fn seize_entry_assets(env: &Env, seized: &Vec<SeizeEntry>) -> Vec<Address> {
    let mut out: Vec<Address> = Vec::new(env);
    for i in 0..seized.len() {
        let entry = validation::expect_invariant(env, seized.get(i));
        if !out.contains(&entry.asset) {
            out.push_back(entry.asset);
        }
    }
    out
}

fn repay_entry_assets(env: &Env, repaid: &Vec<RepayEntry>) -> Vec<Address> {
    let mut out: Vec<Address> = Vec::new(env);
    for i in 0..repaid.len() {
        let entry = validation::expect_invariant(env, repaid.get(i));
        if !out.contains(&entry.asset) {
            out.push_back(entry.asset);
        }
    }
    out
}

fn liquidation_event_context(liquidator: &Address, action: Symbol) -> EventContext {
    EventContext {
        caller: liquidator.clone(),
        event_caller: liquidator.clone(),
        action,
    }
}

fn apply_liquidation_repayments(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    repaid: &Vec<RepayEntry>,
    cache: &mut ControllerCache,
) {
    for i in 0..repaid.len() {
        let entry = validation::expect_invariant(env, repaid.get(i));

        let pool_addr = cache.cached_pool_address(&entry.asset);
        let token = soroban_sdk::token::Client::new(env, &entry.asset);
        token.transfer(liquidator, &pool_addr, &entry.amount);

        let position: DebtPosition =
            (&validation::expect_invariant(env, account.borrow_positions.get(entry.asset.clone())))
                .into();
        repay::execute_repayment(
            env,
            account,
            liquidation_event_context(liquidator, symbol_short!("liq_repay")),
            RepaymentRequest {
                asset: &entry.asset,
                position: &position,
                amount: entry.amount,
                price: Wad::from_raw(entry.feed.price_wad),
            },
            cache,
        );
    }
}

fn apply_liquidation_seizures(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut ControllerCache,
) {
    for i in 0..seized.len() {
        let entry = validation::expect_invariant(env, seized.get(i));

        let position: AccountPosition =
            (&validation::expect_invariant(env, account.supply_positions.get(entry.asset.clone())))
                .into();
        withdraw::execute_withdrawal(
            env,
            account,
            liquidation_event_context(liquidator, symbol_short!("liq_seize")),
            WithdrawalRequest {
                asset: &entry.asset,
                amount: entry.amount,
                position: &position,
                price: Wad::from_raw(entry.feed.price_wad),
            },
            WithdrawFlags {
                is_liquidation: true,
                protocol_fee: entry.protocol_fee,
            },
            cache,
        );
    }
}

pub(crate) fn execute_liquidation(
    env: &Env,
    account: &Account,
    debt_payments: &Vec<Payment>,
    cache: &mut ControllerCache,
) -> LiquidationResult {
    let mut refunds = Vec::new(env);

    let hf = helpers::calculate_health_factor(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    if hf >= Wad::ONE {
        panic_with_error!(env, CollateralError::HealthFactorTooHigh);
    }

    let (total_collateral, total_debt, weighted_coll) = helpers::calculate_account_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    let (proportion_seized, bonus_bounds) =
        calculate_seizure_proportions(env, account, total_collateral, weighted_coll, cache);

    let snap = LiquidationSnapshot {
        total_debt,
        total_collateral,
        weighted_coll,
        proportion_seized,
        hf,
    };

    let (total_debt_payment_usd, repaid_tokens) =
        calculate_repayment_amounts(env, debt_payments, account, &mut refunds, cache);

    let (max_debt_to_repay_usd, _seizure_usd, bonus) =
        calculate_liquidation_amounts(env, &snap, bonus_bounds, total_debt_payment_usd);

    // Full close if residue is dust.
    let max_debt_to_repay_usd = expand_to_full_close_on_dust_residue(
        env,
        cache,
        account,
        DustExpansionInputs {
            snap: &snap,
            bonus,
            payment_ceiling_usd: total_debt_payment_usd,
            repay_usd: max_debt_to_repay_usd,
        },
    );

    let seized_collaterals = calculate_seized_collateral(
        env,
        account,
        total_collateral,
        max_debt_to_repay_usd,
        bonus,
        cache,
    );

    let mut final_repayment_tokens = repaid_tokens;
    if total_debt_payment_usd > max_debt_to_repay_usd {
        let excess_usd = total_debt_payment_usd - max_debt_to_repay_usd;
        process_excess_payment(env, &mut final_repayment_tokens, &mut refunds, excess_usd);
    }

    LiquidationResult {
        seized: seized_collaterals,
        repaid: final_repayment_tokens,
        refunds,
        max_debt_usd: max_debt_to_repay_usd.raw(),
        bonus_bps: bonus.raw(),
    }
}

fn check_bad_debt_after_liquidation(
    env: &Env,
    cache: &mut ControllerCache,
    account_id: u64,
    account: &Account,
) {
    if account.borrow_positions.is_empty() {
        positions::account::cleanup_account_if_empty(env, account, account_id);
        return;
    }

    let (total_collateral_usd, total_debt_usd, _) = helpers::calculate_account_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    let bad_debt_threshold = Wad::from_raw(BAD_DEBT_USD_THRESHOLD);
    if total_debt_usd > total_collateral_usd && total_collateral_usd <= bad_debt_threshold {
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

// Socializes bad debt by seizing collateral and writing off debt.
pub fn clean_bad_debt_standalone(env: &Env, account_id: u64) {
    // The success path removes the account entirely and the failure path
    // reverts atomically, so no upfront `renew_user_account` keep-alive is needed.
    //
    // Bad-debt cleanup is risk-reducing — blocking it on oracle deviation
    // trades a recoverable price-uncertainty event for permanent bad debt.
    // Use the same `Liquidation` policy as `process_liquidation`'s inline
    // cleanup path so the standalone keeper call doesn't revert through
    // the backstop in exactly the oracle conditions that make small bad-
    // debt accounts unprofitable for liquidators to clear.
    let mut cache = ControllerCache::new(env, OraclePolicy::Liquidation);
    let account = storage::get_account(env, account_id);

    if account.borrow_positions.is_empty() {
        panic_with_error!(env, CollateralError::PositionNotFound);
    }

    let (total_collateral_usd, total_debt_usd, _) = helpers::calculate_account_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    let bad_debt_threshold = Wad::from_raw(BAD_DEBT_USD_THRESHOLD);
    if !(total_debt_usd > total_collateral_usd && total_collateral_usd <= bad_debt_threshold) {
        panic_with_error!(env, CollateralError::CannotCleanBadDebt);
    }

    execute_bad_debt_cleanup(
        env,
        &mut cache,
        account_id,
        &account,
        total_debt_usd.raw(),
        total_collateral_usd.raw(),
    );
    cache.flush_isolated_debts();
    cache.emit_market_batch();
}

fn execute_bad_debt_cleanup(
    env: &Env,
    cache: &mut ControllerCache,
    account_id: u64,
    account: &Account,
    total_debt_usd: i128,
    total_collateral_usd: i128,
) {
    for (asset, position) in iter_typed_positions(&account.supply_positions) {
        seize_pool_position(env, cache, AccountPositionType::Deposit, &asset, (&position).into());
    }

    for (asset, position) in iter_debt_positions(&account.borrow_positions) {
        repay::clear_position_isolated_debt(env, &asset, &position, account, cache);
        seize_pool_position(env, cache, AccountPositionType::Borrow, &asset, (&position).into());
    }

    emit_clean_bad_debt(
        env,
        CleanBadDebtEvent {
            account_id,
            total_borrow_usd_wad: total_debt_usd,
            total_collateral_usd_wad: total_collateral_usd,
        },
    );

    positions::account::remove_account(env, account_id);
}

fn seize_pool_position(
    env: &Env,
    cache: &mut ControllerCache,
    side: AccountPositionType,
    asset: &Address,
    position: ScaledPositionRaw,
) {
    let feed = cache.cached_price(asset);
    let pool_addr = cache.cached_pool_address(asset);
    let result = pool_seize_position_call(env, &pool_addr, side, position);
    cache.record_market_update_with_price(&result.market_state, Some(feed.price.raw()));
}
