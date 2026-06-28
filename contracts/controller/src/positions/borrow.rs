//! Borrow and strategy-internal borrow flows.
//!
//! Pipeline: auth → aggregate → cache → configs → validate → settle →
//! post-pool gates → persist → emit. Borrows update scaled debt shares; LTV
//! and health gates run post-pool against the market indexes the pool borrow
//! writes into the cache.

use common::errors::CollateralError;
use common::math::fp::Ray;
use controller_interface::types::{
    Account, AccountPositionType, DebtPosition, Payment, PoolBorrowEntry, PoolPositionMutation,
};
use soroban_sdk::{assert_with_error, contractimpl, Address, Env, Vec};
use stellar_macros::when_not_paused;

use super::{finalize_position_flow, AggregatedConfigs, AggregatedPayments, PositionSides};
use crate::cache::Cache;
use crate::emode;
use crate::events;
use crate::external::pool::{pool_borrow_call, pool_create_strategy_call};
use crate::helpers::update_or_remove_debt_position;
use crate::positions::make_pool_action;
use crate::{helpers::utils, storage, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn borrow(env: Env, caller: Address, account_id: u64, borrows: Vec<(Address, i128)>) {
        process_borrow(&env, &caller, account_id, &borrows);
    }
}

/// Borrows one or more assets; LTV and health validation run post-pool so the
/// valuation reuses the market indexes the borrow itself wrote into the cache.
pub fn process_borrow(env: &Env, caller: &Address, account_id: u64, borrows: &Vec<Payment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);

    let mut cache = Cache::new(env);
    let aggregated = utils::aggregate_positive_payments(env, borrows);

    let configs = AggregatedConfigs::resolve(env, &account, &aggregated, &mut cache);
    validate_borrow(env, &account, &aggregated, &configs, &mut cache);
    settle_borrow(env, caller, &mut account, &aggregated, &configs, &mut cache);

    // A failure in any gate panics and reverts the atomic tx.
    validation::require_post_pool_risk_gates(env, &mut cache, &account);

    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::DEBT,
        false,
    );
}

// Pre-pool gates only: emptiness, position limits, then per-asset market-active
// and borrowability. LTV valuation runs post-pool in
// `require_post_pool_risk_gates` to reuse the borrow's cached market index.
fn validate_borrow(
    env: &Env,
    account: &Account,
    aggregated: &AggregatedPayments,
    configs: &AggregatedConfigs,
    cache: &mut Cache,
) {
    validation::require_non_empty_payments(env, aggregated);
    validation::validate_bulk_position_limits(
        env,
        account,
        AccountPositionType::Borrow,
        aggregated,
    );
    for (asset, _) in aggregated {
        validation::require_market_active(env, cache, &asset);
        let asset_config = configs.get(env, &asset);
        emode::validate_e_mode_asset(env, cache, account.e_mode_category_id, &asset);

        assert_with_error!(
            env,
            asset_config.is_borrowable,
            CollateralError::AssetNotBorrowable
        );
    }
}

fn settle_borrow(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    configs: &AggregatedConfigs,
    cache: &mut Cache,
) {
    // Build the whole batch's entries, make ONE pool call, then merge results
    // input-ordered in one cross-contract frame.
    let mut entries: Vec<PoolBorrowEntry> = Vec::new(env);
    for (asset, amount) in aggregated {
        let borrow_position = account.get_or_create_debt_position(&asset);
        entries.push_back(PoolBorrowEntry {
            action: make_pool_action(&borrow_position, amount, asset.clone()),
        });
    }
    let pool_addr = cache.cached_pool_address();
    let results = pool_borrow_call(env, &pool_addr, caller, &entries);

    for (i, entry) in entries.iter().enumerate() {
        let asset_config = configs.get(env, &entry.action.asset);
        let result = validation::expect_invariant(env, results.get(i as u32));
        merge_borrow_result(
            env,
            account,
            &entry.action.asset,
            events::PositionAction::Borrow,
            &result,
            asset_config.asset_decimals,
            cache,
        );
    }
}

/// Merges one pool borrow result into the account and event buffers.
fn merge_borrow_result(
    env: &Env,
    account: &mut Account,
    asset: &Address,
    action: events::PositionAction,
    result: &PoolPositionMutation,
    asset_decimals: u32,
    cache: &mut Cache,
) {
    let old_scaled = account
        .borrow_positions
        .get(asset.clone())
        .map(|p| Ray::from(p.scaled_amount_ray))
        .unwrap_or(Ray::ZERO);
    let position: DebtPosition = DebtPosition::from(&result.position);
    // dimensional: scaled delta is Ray<Share(asset, borrow)>.
    if let Some(ctx) = cache.emode_usage_mut(account.e_mode_category_id) {
        let delta = position.scaled_amount - old_scaled;
        ctx.apply_borrow_after_pool(env, asset, delta, &result.market_index, asset_decimals);
    }
    cache.put_market_index(asset, &result.market_index);
    // dimensional: actual_amount is Token(asset); index is Ray<Index(asset, borrow)>.
    cache.record_debt_position_update(
        action,
        asset,
        result.market_index.borrow_index_ray,
        result.actual_amount,
        &position,
    );
    update_or_remove_debt_position(account, asset, &position);
}

/// Creates strategy debt in the pool through the shared borrow gates and
/// returns the asset amount received by the controller.
pub fn borrow_for_strategy(
    env: &Env,
    account: &mut Account,
    debt_token: &Address,
    amount: i128,
    cache: &mut Cache,
) -> i128 {
    borrow_strategy_inner(
        env,
        account,
        debt_token,
        amount,
        cache,
        None,
        events::PositionAction::Multiply,
    )
}

/// Zero-fee strategy borrow used by Blend migration.
/// Other strategy borrows defer solvency to `strategy_finalize`.
pub fn borrow_for_migration(
    env: &Env,
    account: &mut Account,
    debt_token: &Address,
    amount: i128,
    cache: &mut Cache,
) -> i128 {
    borrow_strategy_inner(
        env,
        account,
        debt_token,
        amount,
        cache,
        Some(0),
        events::PositionAction::Migrate,
    )
}

/// Shared strategy-borrow body. `fee_override` of `Some(fee)` bypasses the
/// configured flash-loan fee (migration uses `Some(0)`); `None` charges the
/// asset's configured fee.
fn borrow_strategy_inner(
    env: &Env,
    account: &mut Account,
    debt_token: &Address,
    amount: i128,
    cache: &mut Cache,
    fee_override: Option<i128>,
    event_action: events::PositionAction,
) -> i128 {
    let mut payments: AggregatedPayments = Vec::new(env);
    payments.push_back((debt_token.clone(), amount));
    let aggregated = utils::aggregate_positive_payments(env, &payments);
    let configs = AggregatedConfigs::resolve(env, account, &aggregated, cache);
    validate_borrow(env, account, &aggregated, &configs, cache);

    let debt_config = configs.get(env, debt_token);
    let flash_fee =
        fee_override.unwrap_or_else(|| debt_config.flashloan_fee.flash_loan_fee_on(env, amount));
    let borrow_position = account.get_or_create_debt_position(debt_token);

    let pool_addr = cache.cached_pool_address();
    let pool_action = make_pool_action(&borrow_position, amount, debt_token.clone());
    let result = pool_create_strategy_call(
        env,
        &pool_addr,
        &env.current_contract_address(),
        pool_action,
        flash_fee,
    );
    let mutation: PoolPositionMutation = PoolPositionMutation::from(&result);
    merge_borrow_result(
        env,
        account,
        debt_token,
        event_action,
        &mutation,
        debt_config.asset_decimals,
        cache,
    );

    result.amount_received
}
