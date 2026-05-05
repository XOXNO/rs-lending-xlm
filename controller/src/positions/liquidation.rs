use common::constants::{BAD_DEBT_USD_THRESHOLD, WAD};
use common::errors::CollateralError;
use common::events::{emit_clean_bad_debt, CleanBadDebtEvent};
use common::fp::{Bps, Ray, Wad};
use common::types::{
    Account, AccountPosition, AccountPositionType, LiquidationResult, Payment, RepayEntry,
    SeizeEntry,
};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Symbol, Vec};
use stellar_macros::{only_role, when_not_paused};

use super::{repay, withdraw, EventContext};
use crate::cache::ControllerCache;
use crate::positions;
use crate::{helpers, storage, utils, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn liquidate(env: Env, liquidator: Address, account_id: u64, debt_payments: Vec<Payment>) {
        process_liquidation(&env, &liquidator, account_id, &debt_payments);
    }

    #[when_not_paused]
    #[only_role(caller, "KEEPER")]
    pub fn clean_bad_debt(env: Env, caller: Address, account_id: u64) {
        validation::require_not_flash_loaning(&env);

        clean_bad_debt_standalone(&env, account_id);
    }
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

/// Executes a liquidation: verifies HF < 1, computes seizure amounts with the dynamic bonus,
/// repays debt from the liquidator, and seizes proportional collateral.
/// Triggers automatic bad-debt cleanup when residual collateral falls below `BAD_DEBT_USD_THRESHOLD`.
pub fn process_liquidation(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    debt_payments: &Vec<Payment>,
) {
    liquidator.require_auth();
    validation::require_not_flash_loaning(env);
    validation::require_non_empty_payments(env, debt_payments);

    let debt_payment_plan = utils::aggregate_positive_payments(env, debt_payments);

    for (asset, _) in debt_payment_plan.iter() {
        validation::require_asset_supported(env, &asset);
    }

    // The side-map writes below bump account metadata TTLs, so an explicit
    // `bump_account` keep-alive call here is redundant.
    let mut account = storage::get_account(env, account_id);
    let mut cache = ControllerCache::new(env, false);

    // Math phase: decide seizure and repayment amounts.
    //
    // `_refunds` is intentionally discarded here. The pull-model only
    // transfers the post-cap `amount` from `repaid_tokens` below. The cap
    // is enforced at the transfer step itself, so over-collection is
    // impossible. The vector is still produced because the public
    // `liquidation_estimations_detailed` view exposes it as informational
    // metadata for off-chain simulators.
    let result = execute_liquidation(env, &account, &debt_payment_plan, &mut cache);

    validation::require_non_empty_payments(env, &result.repaid);

    apply_liquidation_repayments(
        env,
        liquidator,
        account_id,
        &mut account,
        &result.repaid,
        &mut cache,
    );
    apply_liquidation_seizures(
        env,
        liquidator,
        account_id,
        &mut account,
        &result.seized,
        &mut cache,
    );

    // Liquidation never mutates meta fields (owner, is_isolated, e_mode,
    // mode, isolated_asset). Flush only the two sides; each side write
    // also TTL-bumps meta via `write_side_map`.
    storage::set_supply_positions(env, account_id, &account.supply_positions);
    storage::set_borrow_positions(env, account_id, &account.borrow_positions);

    // Reuse the post-liquidation account snapshot for bad-debt cleanup.
    check_bad_debt_after_liquidation(env, &mut cache, account_id, &account);

    cache.flush_isolated_debts();
}

// ---------------------------------------------------------------------------
// Execution Engine (Math)
// ---------------------------------------------------------------------------

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
    account_id: u64,
    account: &mut Account,
    repaid: &Vec<RepayEntry>,
    cache: &mut ControllerCache,
) {
    for i in 0..repaid.len() {
        let entry = repaid.get(i).unwrap();

        let pool_addr = cache.cached_pool_address(&entry.asset);
        let token = soroban_sdk::token::Client::new(env, &entry.asset);
        token.transfer(liquidator, &pool_addr, &entry.amount);

        let position = account.borrow_positions.get(entry.asset.clone()).unwrap();
        repay::execute_repayment(
            env,
            account,
            account_id,
            &entry.asset,
            liquidation_event_context(liquidator, symbol_short!("liq_repay")),
            &position,
            entry.feed.price_wad,
            entry.amount,
            cache,
        );
    }
}

fn apply_liquidation_seizures(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    account: &mut Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut ControllerCache,
) {
    for i in 0..seized.len() {
        let entry = seized.get(i).unwrap();

        let position = account.supply_positions.get(entry.asset.clone()).unwrap();
        withdraw::execute_withdrawal(
            env,
            account,
            account_id,
            &entry.asset,
            liquidation_event_context(liquidator, symbol_short!("liq_seize")),
            entry.amount,
            &position,
            true,
            entry.protocol_fee,
            entry.feed.price_wad,
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
    if hf >= WAD {
        panic_with_error!(env, CollateralError::HealthFactorTooHigh);
    }

    let (total_collateral, total_debt, weighted_coll) = helpers::calculate_account_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    let (proportion_seized, bonus_params) =
        calculate_seizure_proportions(env, account, total_collateral, weighted_coll, cache);

    let (total_debt_payment_usd, repaid_tokens) =
        calculate_repayment_amounts(env, debt_payments, account, &mut refunds, cache);

    let (max_debt_to_repay_usd, _seizure_usd, bonus) = calculate_liquidation_amounts(
        env,
        total_debt,
        total_collateral,
        weighted_coll,
        proportion_seized,
        bonus_params,
        Wad::from_raw(hf),
        total_debt_payment_usd,
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

fn calculate_seizure_proportions(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    weighted_coll: Wad,
    cache: &mut ControllerCache,
) -> (Wad, (Bps, Bps)) {
    let proportion_seized = if total_collateral > Wad::ZERO {
        weighted_coll.div(env, total_collateral)
    } else {
        Wad::ZERO
    };

    let bonus_params = helpers::get_account_bonus_params(env, cache, &account.supply_positions);

    (proportion_seized, bonus_params)
}

fn calculate_repayment_amounts(
    env: &Env,
    raw_payments: &Vec<Payment>,
    account: &Account,
    refunds: &mut Vec<Payment>,
    cache: &mut ControllerCache,
) -> (Wad, Vec<RepayEntry>) {
    let mut total_repaid_usd = Wad::ZERO;
    let mut repaid_tokens: Vec<RepayEntry> = Vec::new(env);

    let merged = utils::aggregate_positive_payments(env, raw_payments);

    for i in 0..merged.len() {
        let (asset, amount) = merged.get(i).unwrap();
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let position = account
            .borrow_positions
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound));

        let actual_debt = Ray::from_raw(position.scaled_amount_ray)
            .mul(env, Ray::from_raw(market_index.borrow_index_ray))
            .to_asset(feed.asset_decimals);

        let mut payment_amount = amount;
        if payment_amount > actual_debt {
            let excess = payment_amount - actual_debt;
            refunds.push_back((asset.clone(), excess));
            payment_amount = actual_debt;
        }

        let payment_wad = Wad::from_token(payment_amount, feed.asset_decimals);
        let payment_usd = payment_wad.mul(env, Wad::from_raw(feed.price_wad));

        total_repaid_usd += payment_usd;
        repaid_tokens.push_back(RepayEntry {
            asset,
            amount: payment_amount,
            usd_wad: payment_usd.raw(),
            feed,
            market_index,
        });
    }

    (total_repaid_usd, repaid_tokens)
}

fn calculate_liquidation_amounts(
    env: &Env,
    total_debt: Wad,
    total_collateral: Wad,
    weighted_coll: Wad,
    proportion_seized: Wad,
    bonus_params: (Bps, Bps),
    hf: Wad,
    total_payment_usd: Wad,
) -> (Wad, Wad, Bps) {
    let (base_bonus, max_bonus) = bonus_params;
    let (ideal_repayment_usd, bonus) = helpers::estimate_liquidation_amount(
        env,
        total_debt,
        weighted_coll,
        hf,
        base_bonus,
        max_bonus,
        proportion_seized,
        total_collateral,
    );

    let final_repayment_usd = total_payment_usd.min(ideal_repayment_usd);
    let seizure_multiplier = Wad::ONE + bonus.to_wad(env);
    let total_seizure_usd = final_repayment_usd.mul(env, seizure_multiplier);

    (final_repayment_usd, total_seizure_usd, bonus)
}

fn calculate_seized_collateral(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    repayment_usd: Wad,
    bonus: Bps,
    cache: &mut ControllerCache,
) -> Vec<SeizeEntry> {
    let mut seized: Vec<SeizeEntry> = Vec::new(env);
    if total_collateral <= Wad::ZERO {
        return seized;
    }

    let one_plus_bonus = Wad::ONE + bonus.to_wad(env);
    let total_seizure_usd = repayment_usd.mul(env, one_plus_bonus);

    for (asset, position) in account.supply_positions.iter() {
        let feed = cache.cached_price(&asset);
        if feed.price_wad == 0 {
            continue;
        }

        let asset_config = cache.cached_asset_config(&asset);
        let market_index = cache.cached_market_index(&asset);

        let actual_ray = Ray::from_raw(position.scaled_amount_ray)
            .mul(env, Ray::from_raw(market_index.supply_index_ray));
        let actual_amount_wad = actual_ray.to_wad();
        let asset_value = actual_amount_wad.mul(env, Wad::from_raw(feed.price_wad));

        let share = asset_value.div(env, total_collateral);
        let seizure_for_asset_usd = total_seizure_usd.mul(env, share);

        let seizure_amount_wad = seizure_for_asset_usd.div(env, Wad::from_raw(feed.price_wad));
        let seizure_ray = seizure_amount_wad.to_ray();

        if seizure_ray <= Ray::ZERO {
            continue;
        }

        let capped_ray = seizure_ray.min(actual_ray);
        if capped_ray <= Ray::ZERO {
            continue;
        }

        // Split the seized RAY amount into base and bonus before computing
        // the protocol fee. Floor division keeps the base side as the lower
        // bound, so the bonus side captures any rounding remainder.
        let base_ray = capped_ray.div_floor(env, one_plus_bonus.to_ray());
        let bonus_ray = capped_ray - base_ray;
        let protocol_fee =
            Bps::from_raw(asset_config.liquidation_fees_bps).apply_to_ray(env, bonus_ray);
        let capped_amount = capped_ray.to_asset(feed.asset_decimals);
        if capped_amount <= 0 {
            continue;
        }

        seized.push_back(SeizeEntry {
            asset,
            amount: capped_amount,
            protocol_fee: protocol_fee.to_asset(feed.asset_decimals),
            feed,
            market_index,
        });
    }

    seized
}

fn process_excess_payment(
    env: &Env,
    repaid_tokens: &mut Vec<RepayEntry>,
    refunds: &mut Vec<Payment>,
    excess_usd: Wad,
) {
    let mut remaining_excess_usd = excess_usd;

    while remaining_excess_usd > Wad::ZERO && !repaid_tokens.is_empty() {
        let current_index = repaid_tokens.len() - 1;
        let entry = repaid_tokens.get(current_index).unwrap();

        let usd = Wad::from_raw(entry.usd_wad);

        if usd > remaining_excess_usd {
            let ratio = remaining_excess_usd.div(env, usd);
            let refund_amount = Wad::from_token(entry.amount, entry.feed.asset_decimals)
                .mul(env, ratio)
                .to_token(entry.feed.asset_decimals);

            let new_amount = entry.amount - refund_amount;
            // Recompute `new_usd` from `new_amount * price`. Subtracting the
            // excess directly lets the two precision paths drift and leaves
            // the RepayEntry pair inconsistent for downstream consumers.
            let new_amount_wad = Wad::from_token(new_amount, entry.feed.asset_decimals);
            let new_usd = new_amount_wad.mul(env, Wad::from_raw(entry.feed.price_wad));

            refunds.push_back((entry.asset.clone(), refund_amount));
            repaid_tokens.set(
                current_index,
                RepayEntry {
                    asset: entry.asset,
                    amount: new_amount,
                    usd_wad: new_usd.raw(),
                    feed: entry.feed,
                    market_index: entry.market_index,
                },
            );
            remaining_excess_usd = Wad::ZERO;
        } else {
            refunds.push_back((entry.asset, entry.amount));
            repaid_tokens.remove(current_index);
            remaining_excess_usd -= usd;
        }
    }
}

// ---------------------------------------------------------------------------
// Bad debt check and cleanup
// ---------------------------------------------------------------------------

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

/// Socializes the entire position as bad debt: seizes all collateral into protocol revenue
/// and writes off all debt against the supply index. Callable by the KEEPER role.
/// Panics with `CannotCleanBadDebt` when the account does not meet the bad-debt threshold.
pub fn clean_bad_debt_standalone(env: &Env, account_id: u64) {
    // The success path removes the account entirely and the failure path
    // reverts atomically, so no upfront `bump_account` keep-alive is needed.
    let mut cache = ControllerCache::new(env, false);
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
}

fn execute_bad_debt_cleanup(
    env: &Env,
    cache: &mut ControllerCache,
    account_id: u64,
    account: &Account,
    total_debt_usd: i128,
    total_collateral_usd: i128,
) {
    for (asset, position) in account.supply_positions.iter() {
        seize_pool_position(env, cache, AccountPositionType::Deposit, &asset, &position);
    }

    for (asset, position) in account.borrow_positions.iter() {
        repay::clear_position_isolated_debt(env, &asset, &position, account, cache);
        seize_pool_position(env, cache, AccountPositionType::Borrow, &asset, &position);
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
    position: &AccountPosition,
) {
    let feed = cache.cached_price(asset);
    pool_seize_position_call(env, asset, side, position.clone(), feed.price_wad);
}

crate::summarized!(
    pool::seize_position_summary,
    fn pool_seize_position_call(
        env: &Env,
        asset: &Address,
        side: AccountPositionType,
        position: AccountPosition,
        price_wad: i128,
    ) -> AccountPosition {
        let pool_addr = crate::storage::get_market_config(env, asset).pool_address;
        pool_interface::LiquidityPoolClient::new(env, &pool_addr)
            .seize_position(&side, &position, &price_wad)
    }
);
