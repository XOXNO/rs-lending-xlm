use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::math::fp::Ray;
use common::types::{
    Account, AccountPositionType, AssetConfig, AssetConfigRaw,
    Payment, PoolAction, PositionMode,
};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, Address, Env, Map, Vec,
};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::cross_contract::pool::pool_supply_call;
use crate::emode;
use crate::helpers::{
    refresh_supply_risk_params, require_no_supply_dust_for_assets, update_or_remove_supply_position,
};
use crate::oracle::policy::OraclePolicy;
use crate::{storage, utils, validation::*, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        assets: Vec<(Address, i128)>,
    ) -> u64 {
        process_supply(&env, &caller, account_id, e_mode_category, &assets)
    }
}

/// Supplies one or more assets, creating an account when `account_id == 0`.
///
/// Duplicate assets are aggregated before pool calls. The controller stores
/// scaled supply shares returned by pools and emits one position/market batch.
pub fn process_supply(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    assets: &Vec<Payment>,
) -> u64 {
    caller.require_auth();
    require_not_flash_loaning(env);

    let mut cache = Cache::new(env, OraclePolicy::RiskDecreasing);
    // Aggregate once at the entrypoint so every downstream stage, including the
    // post-flight dust scope, operates on the deduped plan.
    let deposit_plan = utils::aggregate_positive_payments(env, assets);
    let (acct_id, mut account) = resolve_supply_account(
        env,
        caller,
        account_id,
        e_mode_category,
        &deposit_plan,
        &mut cache,
    );

    process_deposit(env, caller, &mut account, &deposit_plan, &mut cache);

    // Dust gate scoped to this batch's assets: supply never mutates borrow
    // positions, so pre-existing under-floor positions (price drift) must not block it.
    require_no_supply_dust_for_assets(
        env,
        &mut cache,
        &account,
        &utils::plan_assets(env, &deposit_plan),
    );

    storage::set_supply_positions(env, acct_id, &account.supply_positions);
    cache.emit_position_batch(acct_id, &account);
    cache.emit_market_batch();

    acct_id
}

fn resolve_supply_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    assets: &Vec<Payment>,
    cache: &mut Cache,
) -> (u64, Account) {
    require_non_empty_payments(env, assets);

    if account_id == 0 {
        create_account_for_first_asset(env, caller, e_mode_category, assets, cache)
    } else {
        let account = storage::get_account_supply_only(env, account_id);
        // Zero is the unspecified sentinel; any non-zero value must match the
        // account's stored mode.
        if e_mode_category != 0 && e_mode_category != account.e_mode_category_id {
            panic_with_error!(env, common::errors::EModeError::EModeMismatch);
        }
        (account_id, account)
    }
}

/// Applies an already-deduplicated positive deposit plan to an account.
pub fn process_deposit(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    deposit_plan: &Vec<Payment>,
    cache: &mut Cache,
) {
    let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);

    // Reuse the same e-mode-adjusted config for validation and pool execution.
    let mut effective_configs: Map<Address, AssetConfigRaw> = Map::new(env);
    for (asset, _) in deposit_plan.iter() {
        let cfg = emode::effective_asset_config(env, account, &asset, cache, &e_mode);
        effective_configs.set(asset, (&cfg).into());
    }

    prepare_deposit_plan(env, account, deposit_plan, cache, &effective_configs);
    execute_deposit_plan(
        env,
        caller,
        account,
        deposit_plan,
        cache,
        &effective_configs,
    );
}

fn prepare_deposit_plan(
    env: &Env,
    account: &Account,
    assets: &Vec<Payment>,
    cache: &mut Cache,
    effective_configs: &Map<Address, AssetConfigRaw>,
) {
    validate_bulk_position_limits(env, account, AccountPositionType::Deposit, assets);
    validate_bulk_isolation(env, account, assets, cache);

    for (asset, _) in assets {
        require_market_active(env, cache, &asset);

        let asset_config: AssetConfig =
            (&expect_invariant(env, effective_configs.get(asset.clone()))).into();

        emode::validate_e_mode_asset(env, cache, account.e_mode_category_id, &asset);
        emode::ensure_e_mode_compatible_with_asset(env, &asset_config, account.e_mode_category_id);

        assert_with_error!(
            env,
            asset_config.can_supply(),
            CollateralError::NotCollateral
        );

        emode::validate_isolated_collateral(env, account, &asset, &asset_config);
    }
}

fn execute_deposit_plan(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    assets: &Vec<Payment>,
    cache: &mut Cache,
    effective_configs: &Map<Address, AssetConfigRaw>,
) {
    // Transfer every asset in and build the entries, make ONE pool call, then
    // merge results input-ordered — one cross-contract frame instead of one
    // per asset. All transfers precede the pool call; the tx stays atomic.
    let pool_addr = cache.cached_pool_address();
    let mut entries: Vec<common::types::PoolSupplyEntry> = Vec::new(env);
    for (asset, amount_in) in assets {
        let asset_config: AssetConfig =
            (&expect_invariant(env, effective_configs.get(asset.clone()))).into();
        utils::transfer_amount(
            env,
            &asset,
            caller,
            &pool_addr,
            amount_in,
            GenericError::AmountMustBePositive,
        );
        let position = account.get_or_create_supply_position(&asset, &asset_config);
        entries.push_back(common::types::PoolSupplyEntry {
            action: PoolAction {
                position: (&position).into(),
                amount: amount_in,
                asset: asset.clone(),
            },
            supply_cap: asset_config.supply_cap,
        });
    }
    let results = pool_supply_call(env, &pool_addr, &entries);

    for (i, entry) in entries.iter().enumerate() {
        let result = expect_invariant(env, results.get(i as u32));
        let asset = &entry.action.asset;
        let asset_config: AssetConfig =
            (&expect_invariant(env, effective_configs.get(asset.clone()))).into();

        let mut position = account.get_or_create_supply_position(asset, &asset_config);
        refresh_supply_risk_params(env, cache, account, asset, &mut position, &asset_config);
        // Merge ONLY the scaled share back; the pool does not echo collateral
        // risk params, so preserve the ones the controller already holds.
        position.scaled_amount = Ray::from(result.position.scaled_amount_ray);
        cache.record_market_update(&result.market_state);

        // Emit with the exact supply index the pool used, not a re-read.
        cache.record_position_update(
            common::events::PositionAction::Supply,
            asset,
            result.market_index.supply_index_ray,
            entry.action.amount,
            &position,
        );

        // Storage is written once after the whole supply batch completes.
        update_or_remove_supply_position(account, asset, &position);
    }
}

fn validate_bulk_isolation(env: &Env, account: &Account, assets: &Vec<Payment>, cache: &mut Cache) {
    if assets.len() <= 1 {
        return;
    }
    let (first_asset, _) = expect_invariant(env, assets.get(0));
    let first_config = cache.cached_asset_config(&first_asset);
    if account.is_isolated || first_config.is_isolated_asset {
        panic_with_error!(env, FlashLoanError::BulkSupplyNoIso);
    }
}

fn create_account_for_first_asset(
    env: &Env,
    caller: &Address,
    e_mode_category: u32,
    assets: &Vec<Payment>,
    cache: &mut Cache,
) -> (u64, Account) {
    let (first_asset, _) = expect_invariant(env, assets.get(0));
    let first_config = cache.cached_asset_config(&first_asset);
    let is_isolated = first_config.is_isolated_asset;
    let isolated_asset = if is_isolated {
        Some(first_asset.clone())
    } else {
        None
    };
    crate::helpers::create_account(
        env,
        caller,
        e_mode_category,
        PositionMode::Normal,
        is_isolated,
        isolated_asset,
    )
}
