//! Borrow and strategy-internal borrow flows.
//!
//! Borrows use `OraclePolicy::RiskIncreasing`, update scaled debt shares, and
//! increment isolated-debt counters when the account is isolated.

use common::errors::{CollateralError, EModeError};
use common::types::{
    Account, AccountPositionType, AssetConfig, AssetConfigRaw, DebtPosition, Payment, PoolAction,
    PoolBorrowEntry, PoolPositionMutation,
};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Env, Map, Vec};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::cross_contract::pool::{pool_borrow_call, pool_create_strategy_call};
use crate::emode;
use crate::helpers::{require_no_borrow_dust_for_assets, update_or_remove_debt_position};
use crate::oracle::policy::OraclePolicy;
use crate::positions::isolated_debt::add_isolated_debt;
use crate::{storage, utils, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn borrow(env: Env, caller: Address, account_id: u64, borrows: Vec<(Address, i128)>) {
        process_borrow(&env, &caller, account_id, &borrows);
    }
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
    let mut plan: Vec<Payment> = Vec::new(env);
    plan.push_back((debt_token.clone(), amount));
    let effective_configs = super::effective_configs_for_plan(env, account, &plan, cache);
    prepare_borrow_plan(env, account, &plan, &effective_configs, cache);

    let debt_config = super::effective_config(env, &effective_configs, debt_token);
    let flash_fee = debt_config.flashloan_fee.apply_to(env, amount);
    let borrow_position = account.get_or_create_debt_position(debt_token);

    let pool_addr = cache.cached_pool_address();
    let action = PoolAction {
        position: (&borrow_position).into(),
        amount,
        asset: debt_token.clone(),
    };
    let result = pool_create_strategy_call(
        env,
        &pool_addr,
        &env.current_contract_address(),
        action,
        flash_fee,
        debt_config.borrow_cap,
    );
    let mutation: PoolPositionMutation = (&result).into();
    merge_borrow_result(
        account,
        debt_token,
        common::events::PositionAction::Multiply,
        &mutation,
        cache,
    );

    result.amount_received
}

/// Borrows one or more assets after LTV pre-checks and final health validation.
pub fn process_borrow(env: &Env, caller: &Address, account_id: u64, borrows: &Vec<Payment>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);

    validation::require_account_owner_match(env, &account, caller);

    let mut cache = Cache::new(env, OraclePolicy::RiskIncreasing);
    // Dedup once at the entrypoint so every downstream stage, including the
    // post-flight dust scope, sees one entry per asset.
    let plan = utils::aggregate_positive_payments(env, borrows);

    process_borrow_plan(env, caller, &mut account, &plan, &mut cache);

    // require_within_ltv and require_healthy_account price the full
    // supply+borrow set before the HF-body prefetch in
    // calculate_account_totals_body can fire; prefetch the union here so those
    // reads hit the cache. Plan assets are already in borrow_positions after
    // the pool call.
    let mut priced_assets = account.supply_positions.keys();
    priced_assets.append(&account.borrow_positions.keys());
    crate::oracle::prefetch_redstone_feeds(&mut cache, &priced_assets);

    // LTV gate runs post-pool so collateral and debt valuation reuse the market
    // indexes the pool borrow just wrote into the cache, sparing a redundant
    // get_sync_data read. A failure here panics and reverts the atomic tx.
    validation::require_within_ltv(env, &mut cache, &account);
    validation::require_healthy_account(env, &mut cache, &account);
    // Scope the dust gate to borrowed assets only: borrow never mutates supply,
    // so it must not be blocked by pre-existing positions that drifted under the floor.
    require_no_borrow_dust_for_assets(
        env,
        &mut cache,
        &account,
        &utils::plan_assets(env, &plan),
    );

    storage::set_debt_positions(env, account_id, &account.borrow_positions);
    cache.flush_isolated_debts();
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();
}

/// Executes a validated, deduplicated borrow plan (`aggregate_positive_payments`
/// output): mutates the account, calls pools, records mutations for events.
fn process_borrow_plan(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    plan: &Vec<Payment>,
    cache: &mut Cache,
) {
    let effective_configs = super::effective_configs_for_plan(env, account, plan, cache);

    prepare_borrow_plan(env, account, plan, &effective_configs, cache);
    execute_borrow_plan(env, caller, account, plan, &effective_configs, cache);
}

/// Account-level borrowability for one asset: isolation, e-mode, borrow flag.
fn validate_asset_borrowable(
    env: &Env,
    account: &Account,
    asset: &Address,
    asset_config: &AssetConfig,
    cache: &mut Cache,
) {
    if account.is_isolated && !asset_config.can_borrow_in_isolation() {
        panic_with_error!(env, EModeError::NotBorrowableIsolation);
    }

    emode::validate_e_mode_asset(env, cache, account.e_mode_category_id, asset);
    emode::ensure_e_mode_compatible_with_asset(env, asset_config, account.e_mode_category_id);

    assert_with_error!(
        env,
        asset_config.is_borrowable,
        CollateralError::AssetNotBorrowable
    );
}

// Pre-pool gates only: emptiness, position limits, siloed set, then per-asset
// market-active, borrowability, and isolated-debt ceilings. LTV valuation runs
// post-pool in `require_within_ltv` to reuse the borrow's cached market index.
fn prepare_borrow_plan(
    env: &Env,
    account: &Account,
    plan: &Vec<Payment>,
    effective_configs: &Map<Address, AssetConfigRaw>,
    cache: &mut Cache,
) {
    validation::require_non_empty_payments(env, plan);
    validation::validate_bulk_position_limits(env, account, AccountPositionType::Borrow, plan);
    validate_siloed_borrow_set(env, account, plan, cache);

    for (asset, amount) in plan {
        validation::require_market_active(env, cache, &asset);
        let asset_config = super::effective_config(env, effective_configs, &asset);
        validate_asset_borrowable(env, account, &asset, &asset_config, cache);
        add_isolated_debt(env, cache, account, &asset, amount);
    }
}

/// Siloed assets must be an account's only borrow; checks the union of
/// existing debt and the incoming plan.
fn validate_siloed_borrow_set(env: &Env, account: &Account, plan: &Vec<Payment>, cache: &mut Cache) {
    let mut union: Vec<Address> = Vec::new(env);
    for asset in account.borrow_positions.keys() {
        utils::push_unique_address(&mut union, asset);
    }
    for (asset, _) in plan {
        utils::push_unique_address(&mut union, asset);
    }

    if union.len() <= 1 {
        return;
    }

    for asset in union {
        let config = cache.cached_asset_config(&asset);
        assert_with_error!(
            env,
            !config.is_siloed_borrowing,
            CollateralError::NotBorrowableSiloed
        );
    }
}

fn execute_borrow_plan(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    plan: &Vec<Payment>,
    effective_configs: &Map<Address, AssetConfigRaw>,
    cache: &mut Cache,
) {
    // Build the whole plan's entries, make ONE pool call, then merge results
    // input-ordered — one cross-contract frame instead of one per asset.
    let mut entries: Vec<PoolBorrowEntry> = Vec::new(env);
    for (asset, amount) in plan {
        let asset_config = super::effective_config(env, effective_configs, &asset);
        let borrow_position = account.get_or_create_debt_position(&asset);
        entries.push_back(PoolBorrowEntry {
            action: PoolAction {
                position: (&borrow_position).into(),
                amount,
                asset: asset.clone(),
            },
            borrow_cap: asset_config.borrow_cap,
        });
    }
    let pool_addr = cache.cached_pool_address();
    let results = pool_borrow_call(env, &pool_addr, caller, &entries);

    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        merge_borrow_result(
            account,
            &entry.action.asset,
            common::events::PositionAction::Borrow,
            &result,
            cache,
        );
    }
}

/// Merges one pool borrow result into the account and event buffers.
fn merge_borrow_result(
    account: &mut Account,
    asset: &Address,
    action: common::events::PositionAction,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    cache.record_market_update(&result.market_state);
    let position: DebtPosition = (&result.position).into();
    cache.record_debt_position_update(
        action,
        asset,
        result.market_index.borrow_index_ray,
        result.actual_amount,
        &position,
    );
    update_or_remove_debt_position(account, asset, &position);
}
