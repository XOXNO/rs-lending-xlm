//! Liquidation and keeper bad-debt cleanup.
//!
//! Liquidation requires health factor below one, prices with `OraclePolicy::Liquidation`,
//! repays debt, seizes collateral, and refunds payment above the close amount. Bad-debt
//! cleanup socializes residual debt only when collateral is below the USD threshold.

use common::errors::{CollateralError, GenericError};
use common::events::CleanBadDebtEvent;
use common::math::fp::Wad;
use common::types::{
    Account, AccountPosition, AccountPositionType, DebtPosition, LiquidationResult, Payment,
    PoolAction, PoolWithdrawEntry, RepayEntry, ScaledPositionRaw, SeizeEntry,
};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Env, Vec};
use stellar_macros::{only_role, when_not_paused};

use super::liquidation_math::*;
use super::{repay, withdraw};
use crate::cache::Cache;
use crate::cross_contract::pool::pool_seize_position_call;
use crate::cross_contract::sac::sac_transfer_call;
use crate::helpers::{require_no_borrow_dust_for_assets, require_no_supply_dust_for_assets};
use crate::oracle::policy::OraclePolicy;
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

    let plan = utils::aggregate_positive_payments(env, debt_payments);

    // Liquidation policy: seizure needs a defensible price, so it denies every
    // loosening (stale/deviation/TWAP). Beyond the last tolerance band it
    // reverts (`UnsafePriceNotAllowed`) rather than seize at a price only one
    // source corroborates; inside the bands the standard primary/midpoint
    // selection applies.
    let mut cache = Cache::new(env, OraclePolicy::Liquidation);

    for (asset, _) in plan.iter() {
        validation::require_asset_supported(env, &mut cache, &asset);
    }

    let result = execute_liquidation(env, &account, &plan, &mut cache);

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
    let will_socialize = is_socializable_bad_debt(post_total_debt, post_total_coll);
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
        will_socialize,
    );

    cache.flush_isolated_debts();
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();
}

/// Computes the liquidation outcome (repayments, seizures, refunds) from the
/// account snapshot and the liquidator's payment plan; mutates nothing.
pub(crate) fn execute_liquidation(
    env: &Env,
    account: &Account,
    plan: &Vec<Payment>,
    cache: &mut Cache,
) -> LiquidationResult {
    let mut refunds = Vec::new(env);

    // One totals pass feeds both the HF gate and the snapshot; the inlined HF
    // mirrors calculate_health_factor, including the debt-free early panic
    // that prices nothing.
    if account.borrow_positions.is_empty() {
        panic_with_error!(env, CollateralError::HealthFactorTooHigh);
    }
    let (total_collateral, total_debt, weighted_coll) = helpers::calculate_account_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    let hf = if total_debt == Wad::ZERO {
        Wad::from(i128::MAX)
    } else {
        weighted_coll.div_floor(env, total_debt)
    };
    assert_with_error!(env, hf < Wad::ONE, CollateralError::HealthFactorTooHigh);

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
        calculate_repayment_amounts(env, plan, account, &mut refunds, cache);

    let (max_debt_to_repay_usd, bonus) =
        calculate_liquidation_amounts(env, &snap, bonus_bounds, total_debt_payment_usd);

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
        actions.push_back(PoolAction {
            position: (&position).into(),
            amount: entry.amount,
            asset: entry.asset.clone(),
        });
    }
    repay::settle_repay_actions(
        env,
        account,
        liquidator,
        common::events::PositionAction::LiqRepay,
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
            action: PoolAction {
                position: (&position).into(),
                amount: entry.amount,
                asset: entry.asset.clone(),
            },
            protocol_fee: entry.protocol_fee,
        });
    }
    withdraw::settle_withdraw_entries(
        env,
        account,
        liquidator,
        true,
        common::events::PositionAction::LiqSeize,
        &entries,
        cache,
    );
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

    if !is_socializable_bad_debt(total_debt_usd, total_collateral_usd) {
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
    let pool_addr = cache.cached_pool_address();
    let result = pool_seize_position_call(env, &pool_addr, asset, side, position);
    cache.record_market_update(&result.market_state);
}
