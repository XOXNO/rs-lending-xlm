//! Borrow flows. Post-pool risk gates use pool-returned indexes.

use common::math::fp::{Bps, Ray};
use common::types::{
    Account, AccountPositionType, DebtPosition, HubAssetKey, PoolBorrowEntry, PoolPositionMutation,
};
use soroban_sdk::{contractimpl, Address, Env, Vec};
use stellar_macros::when_not_paused;

use crate::account::{require_owner_or_delegate, update_or_remove_debt_position};
use crate::context::Cache;
use crate::events;
use crate::external::pool::{pool_borrow_call, pool_create_strategy_call};
use crate::positions::{
    finalize_position_flow, validate_position_entry_gates, AggregatedPayments, PositionSides,
};
use crate::positions::{make_pool_action, HubPayment};
use crate::{
    payments as utils, risk::validation, storage, Controller, ControllerArgs, ControllerClient,
};

#[contractimpl]
impl Controller {
    /// Borrows one or more assets against `account_id`, sending proceeds to `to`
    /// (default `caller`). Re-checks account health on pool-returned indexes.
    ///
    /// # Arguments
    /// * `caller` - the account owner or an active delegate; must authorize.
    /// * `borrows` - `(hub-asset, amount)` legs; amounts must be positive.
    /// * `to` - proceeds recipient; defaults to `caller`.
    ///
    /// # Errors
    /// * `NotAuthorized` - `caller` is neither the account owner nor an active delegate.
    /// * `FlashLoanOngoing` - a flash loan or strategy is mid-execution.
    /// * Entry gates: `HubNotActive`, `PairNotActive`, `AssetNotInSpoke`,
    ///   `SpokeAssetPaused`, `SpokeAssetFrozen`, `AssetNotBorrowable`, or
    ///   `PositionLimitExceeded`.
    /// * `SpokeBorrowCapReached` - the borrow would exceed the spoke borrow cap.
    /// * Post-pool risk gates: `InsufficientCollateral` (LTV / health factor) or
    ///   `MinBorrowCollateralNotMet`.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * A position-batch event summarizing the account's updated debt legs.
    #[when_not_paused]
    pub fn borrow(
        env: Env,
        caller: Address,
        account_id: u64,
        borrows: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) {
        process_borrow(&env, &caller, account_id, &borrows, to);
    }
}

/// Borrows one or more assets.
pub fn process_borrow(
    env: &Env,
    caller: &Address,
    account_id: u64,
    borrows: &Vec<HubPayment>,
    to: Option<Address>,
) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);
    require_owner_or_delegate(env, account_id, caller, &account.owner);

    let recipient = to.unwrap_or_else(|| caller.clone());

    let mut cache = Cache::new(env);
    let aggregated = utils::aggregate_positive_payments(env, borrows);

    validate_position_entry_gates(
        env,
        &account,
        &aggregated,
        &mut cache,
        AccountPositionType::Borrow,
    );
    settle_borrow(env, &recipient, &mut account, &aggregated, &mut cache);

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

/// Builds the batch's borrow entries, makes one pool call, and merges results.
fn settle_borrow(
    env: &Env,
    recipient: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    // Build the whole batch's entries, make ONE pool call, then merge results
    // input-ordered in one cross-contract frame.
    let mut entries: Vec<PoolBorrowEntry> = Vec::new(env);
    for (hub_asset, amount) in aggregated {
        let borrow_position = account.get_or_create_debt_position(&hub_asset);
        entries.push_back(PoolBorrowEntry {
            action: make_pool_action(&borrow_position, amount, hub_asset.clone()),
        });
    }
    let pool_addr = cache.cached_pool_address();
    let results = pool_borrow_call(env, &pool_addr, recipient, &entries);

    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        merge_borrow_result(
            env,
            account,
            &entry.action.hub_asset,
            events::PositionAction::Borrow,
            &result,
            cache,
        );
    }
}

/// Merges one pool borrow result into the account and event buffers.
fn merge_borrow_result(
    env: &Env,
    account: &mut Account,
    hub_asset: &HubAssetKey,
    action: events::PositionAction,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    let old_scaled = account
        .borrow_positions
        .get(hub_asset.clone())
        .map(|p| Ray::from(p.scaled_amount))
        .unwrap_or(Ray::ZERO);
    let position: DebtPosition = DebtPosition::from(&result.position);
    // Spoke-cap accounting needs the asset decimals; source them from the active
    // market's oracle config before re-borrowing `cache`.
    let asset_decimals = cache.cached_asset_oracle(&hub_asset.asset).asset_decimals;
    let ctx = cache.require_spoke_usage_context(account.spoke_id);
    let delta = position.scaled_amount - old_scaled;
    ctx.apply_borrow_after_pool(env, hub_asset, delta, &result.market_index, asset_decimals);
    cache.put_market_index(hub_asset, &result.market_index);
    cache.record_debt_position_update(
        action,
        hub_asset,
        result.market_index.borrow_index,
        result.actual_amount,
        &position,
    );
    update_or_remove_debt_position(account, hub_asset, &position);
}

/// Creates strategy debt on `hub_debt`'s market through the shared borrow gates
/// and returns the asset amount received by the controller.
///
/// # Security Warning
/// * Performs no `require_auth`: caller authorization is enforced by the strategy
///   entrypoint that invokes it, and post-borrow solvency is deferred to the
///   strategy's finalize step. Never call from an un-authorized context.
pub fn borrow_for_strategy(
    env: &Env,
    account: &mut Account,
    hub_debt: &HubAssetKey,
    amount: i128,
    cache: &mut Cache,
) -> i128 {
    borrow_strategy_inner(
        env,
        account,
        hub_debt,
        amount,
        cache,
        None,
        events::PositionAction::Multiply,
    )
}

/// Zero-fee strategy borrow used by Blend migration. The caller supplies the
/// explicit `hub_debt` coordinate. Other strategy borrows defer solvency to
/// `strategy_finalize`.
///
/// # Security Warning
/// * Performs no `require_auth`: authorization is enforced by the migration
///   entrypoint that invokes it.
pub fn borrow_for_migration(
    env: &Env,
    account: &mut Account,
    hub_debt: &HubAssetKey,
    amount: i128,
    cache: &mut Cache,
) -> i128 {
    borrow_strategy_inner(
        env,
        account,
        hub_debt,
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
    hub_debt: &HubAssetKey,
    amount: i128,
    cache: &mut Cache,
    fee_override: Option<i128>,
    event_action: events::PositionAction,
) -> i128 {
    let hub_debt = hub_debt.clone();
    let mut payments: AggregatedPayments = Vec::new(env);
    payments.push_back((hub_debt.clone(), amount));
    let aggregated = utils::aggregate_positive_payments(env, &payments);
    validate_position_entry_gates(
        env,
        account,
        &aggregated,
        cache,
        AccountPositionType::Borrow,
    );

    // Flash-loan parameters live on the pool market params, not the spoke config.
    let flash_fee = fee_override.unwrap_or_else(|| {
        let fee_bps = cache.cached_pool_sync_data(&hub_debt).params.flashloan_fee;
        Bps::from(i128::from(fee_bps)).flash_loan_fee_on(env, amount)
    });
    let borrow_position = account.get_or_create_debt_position(&hub_debt);

    let pool_addr = cache.cached_pool_address();
    let pool_action = make_pool_action(&borrow_position, amount, hub_debt.clone());
    let result = pool_create_strategy_call(
        env,
        &pool_addr,
        &env.current_contract_address(),
        pool_action,
        flash_fee,
    );
    let mutation: PoolPositionMutation = PoolPositionMutation::from(&result);
    merge_borrow_result(env, account, &hub_debt, event_action, &mutation, cache);

    result.amount_received
}
