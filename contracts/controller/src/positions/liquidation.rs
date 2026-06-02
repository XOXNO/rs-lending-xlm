//! Liquidation and keeper bad-debt cleanup.
//!
//! Liquidation requires health factor below one, prices with `OraclePolicy::Liquidation`,
//! repays debt, seizes collateral, and refunds payment above the close amount. Bad-debt
//! cleanup socializes residual debt only when collateral is below the USD threshold.

use common::constants::BAD_DEBT_USD_THRESHOLD;
use common::errors::{CollateralError, GenericError};
use common::events::CleanBadDebtEvent;
use common::math::fp::Wad;
use common::types::{
    Account, AccountPosition, AccountPositionType, DebtPosition, LiquidationResult, Payment,
    RepayEntry, ScaledPositionRaw, SeizeEntry,
};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, symbol_short, Address, Env, Symbol, Vec,
};
use stellar_macros::{only_role, when_not_paused};

use super::liquidation_math::*;
use super::repay::RepaymentRequest;
use super::withdraw::{WithdrawFlags, WithdrawalRequest};
use super::{repay, withdraw};
use crate::cache::Cache;
use crate::cross_contract::pool::pool_seize_position_call;
use crate::cross_contract::sac::sac_transfer_call;
use crate::helpers::{require_no_borrow_dust_for_assets, require_no_supply_dust_for_assets};
use crate::oracle::policy::OraclePolicy;
use crate::storage::{iter_debt_positions, iter_typed_positions};
use crate::utils::EventContext;
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

/// Liquidates an underwater account using protocol prices, bonus math, and pool calls.
pub fn process_liquidation(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    debt_payments: &Vec<Payment>,
) {
    liquidator.require_auth();
    validation::require_not_flash_loaning(env);
    validation::require_non_empty_payments(env, debt_payments);

    let mut account = storage::get_account(env, account_id);

    // Reject self-liquidation.
    assert_with_error!(
        env,
        account.owner != *liquidator,
        GenericError::AccountNotInMarket
    );

    let debt_payment_plan = utils::aggregate_positive_payments(env, debt_payments);

    // Liquidation policy: seizure fairness needs a defensible price, so it keeps
    // strict staleness/deviation gates but prefers the aggregator (CEX spot) on
    // tolerance breach; the band check still blocks manipulation (ADR 0003).
    let mut cache = Cache::new(env, OraclePolicy::Liquidation);

    for (asset, _) in debt_payment_plan.iter() {
        validation::require_asset_supported(env, &mut cache, &asset);
    }

    let result = execute_liquidation(env, &account, &debt_payment_plan, &mut cache);

    validation::require_non_empty_payments(env, &result.repaid);

    apply_liquidation_repayments(env, liquidator, &mut account, &result.repaid, &mut cache);
    apply_liquidation_seizures(env, liquidator, &mut account, &result.seized, &mut cache);

    // Per-leg dust gate scoped to the assets this liquidation touched (seized supply
    // + repaid debt); positions that drifted under floor on price moves must not block it.
    let (post_total_coll, post_total_debt, _) = helpers::calculate_account_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    let will_socialize = is_socializable_bad_debt(
        post_total_debt,
        post_total_coll,
        Wad::from(BAD_DEBT_USD_THRESHOLD),
    );
    if !will_socialize {
        let seized_assets = unique_assets(env, &result.seized, |e| e.asset.clone());
        let repaid_assets = unique_assets(env, &result.repaid, |e| e.asset.clone());
        require_no_supply_dust_for_assets(env, &mut cache, &account, &seized_assets);
        require_no_borrow_dust_for_assets(env, &mut cache, &account, &repaid_assets);
    }

    storage::set_supply_positions(env, account_id, &account.supply_positions);
    storage::set_debt_positions(env, account_id, &account.borrow_positions);

    // Reuse the post-liquidation account snapshot for bad-debt cleanup.
    check_bad_debt_after_liquidation(
        env,
        &mut cache,
        account_id,
        &account,
        post_total_coll,
        post_total_debt,
    );

    cache.flush_isolated_debts();
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();
}

// Order-preserving unique asset list from any entry vector keyed by `asset_of`.
fn unique_assets<T>(env: &Env, entries: &Vec<T>, asset_of: impl Fn(&T) -> Address) -> Vec<Address>
where
    T: soroban_sdk::TryFromVal<Env, soroban_sdk::Val> + soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
{
    let mut out: Vec<Address> = Vec::new(env);
    for i in 0..entries.len() {
        let entry = validation::expect_invariant(env, entries.get(i));
        utils::push_unique_address(&mut out, asset_of(&entry));
    }
    out
}

fn liquidation_event_context(liquidator: &Address, action: Symbol) -> EventContext {
    EventContext {
        caller: liquidator.clone(),
        action,
    }
}

fn apply_liquidation_repayments(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    repaid: &Vec<RepayEntry>,
    cache: &mut Cache,
) {
    for entry in repaid.iter() {
        let pool_addr = cache.cached_pool_address(&entry.asset);
        // All SAC transfers go through the wrapper so the harness can replace it.
        sac_transfer_call(env, &entry.asset, liquidator, &pool_addr, &entry.amount);

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
                price: Wad::from(entry.feed.price_wad),
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
    cache: &mut Cache,
) {
    for entry in seized.iter() {
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
                price: Wad::from(entry.feed.price_wad),
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
    cache: &mut Cache,
) -> LiquidationResult {
    let mut refunds = Vec::new(env);

    let hf = helpers::calculate_health_factor(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    assert_with_error!(env, hf < Wad::ONE, CollateralError::HealthFactorTooHigh);

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

    let (max_debt_to_repay_usd, bonus) =
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
    cache: &mut Cache,
    account_id: u64,
    account: &Account,
    total_collateral_usd: Wad,
    total_debt_usd: Wad,
) {
    if account.borrow_positions.is_empty() {
        helpers::cleanup_account_if_empty(env, account, account_id);
        return;
    }

    if is_socializable_bad_debt(
        total_debt_usd,
        total_collateral_usd,
        Wad::from(BAD_DEBT_USD_THRESHOLD),
    ) {
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
    // Success removes the account; failure reverts atomically, so no upfront keep-alive.
    // Cleanup is risk-reducing, so it uses the same `Liquidation` policy as the inline
    // path — blocking on oracle deviation would trade recoverable uncertainty for
    // permanent bad debt in exactly the conditions these accounts need clearing.
    let mut cache = Cache::new(env, OraclePolicy::Liquidation);
    let account = storage::get_account(env, account_id);

    assert_with_error!(
        env,
        !account.borrow_positions.is_empty(),
        CollateralError::PositionNotFound
    );

    let (total_collateral_usd, total_debt_usd, _) = helpers::calculate_account_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    if !is_socializable_bad_debt(
        total_debt_usd,
        total_collateral_usd,
        Wad::from(BAD_DEBT_USD_THRESHOLD),
    ) {
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
        crate::positions::isolated_debt::clear_position_isolated_debt(
            env, &asset, &position, account, cache,
        );
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
    let feed = cache.cached_price(asset);
    let pool_addr = cache.cached_pool_address(asset);
    let result = pool_seize_position_call(env, &pool_addr, side, position);
    cache.record_market_update_with_price(&result.market_state, Some(feed.price.raw()));
}
