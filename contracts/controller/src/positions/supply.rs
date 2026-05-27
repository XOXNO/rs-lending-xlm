use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, AccountPositionType, AssetConfig, AssetConfigRaw, MarketIndex,
    Payment, PositionMode,
};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, symbol_short, Address, Env, Map, Vec,
};
use stellar_macros::when_not_paused;

use crate::cache::ControllerCache;
use crate::cross_contract::pool::pool_supply_call;
use crate::emode;
use crate::helpers::{require_no_supply_dust_for_assets, update_or_remove_supply_position};
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

    let mut cache = ControllerCache::new(env, OraclePolicy::RiskDecreasing);
    // Aggregate once at the entrypoint; downstream stages — including the
    // post-flight dust scope — operate on the deduped plan.
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

    // Dust gate is scoped to the assets supplied in this batch — supply
    // never mutates borrow positions and must not be blocked by pre-existing
    // positions whose USD value drifted under the floor from price moves.
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
    cache: &mut ControllerCache,
) -> (u64, Account) {
    require_non_empty_payments(env, assets);

    if account_id == 0 {
        create_account_for_first_asset(env, caller, e_mode_category, assets, cache)
    } else {
        let account = storage::get_account_supply_only(env, account_id);
        // Non-zero `e_mode_category` must match the account's stored mode;
        // zero is the unspecified sentinel.
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
    cache: &mut ControllerCache,
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
    cache: &mut ControllerCache,
    effective_configs: &Map<Address, AssetConfigRaw>,
) {
    validate_bulk_position_limits(env, account, AccountPositionType::Deposit, assets);
    validate_bulk_isolation(env, account, assets, cache);

    for (asset, _) in assets {
        require_market_active(env, cache, &asset);

        let asset_config: AssetConfig =
            (&expect_invariant(env, effective_configs.get(asset.clone()))).into();

        emode::validate_e_mode_asset(env, cache, account.e_mode_category_id, &asset, true);
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
    cache: &mut ControllerCache,
    effective_configs: &Map<Address, AssetConfigRaw>,
) {
    for (asset, amount_in) in assets {
        let asset_config: AssetConfig =
            (&expect_invariant(env, effective_configs.get(asset.clone()))).into();

        update_deposit_position(
            env,
            account,
            DepositRequest {
                asset: &asset,
                amount: amount_in,
                asset_config: &asset_config,
            },
            caller,
            cache,
        );
    }
}

/// Per-asset supply inputs used after e-mode config resolution.
pub struct DepositRequest<'a> {
    pub asset: &'a Address,
    pub amount: i128,
    pub asset_config: &'a AssetConfig,
}

/// Pulls tokens into the pool and merges the returned scaled supply share.
pub fn update_deposit_position(
    env: &Env,
    account: &mut Account,
    req: DepositRequest<'_>,
    caller: &Address,
    cache: &mut ControllerCache,
) -> AccountPosition {
    let mut position = account.get_or_create_supply_position(req.asset, req.asset_config);

    // Liquidation threshold is updated only by the keeper propagation path.
    if position.loan_to_value != req.asset_config.loan_to_value {
        position.loan_to_value = req.asset_config.loan_to_value;
    }
    if position.liquidation_bonus != req.asset_config.liquidation_bonus {
        position.liquidation_bonus = req.asset_config.liquidation_bonus;
    }

    let market_update = update_market_position(
        env,
        cache,
        &mut position,
        req.asset,
        req.amount,
        req.asset_config,
        caller,
    );

    // Event (supply uses supply_index_ray). The pool synced indexes and
    // returned the exact market index used for this mutation.
    cache.record_position_update(
        symbol_short!("supply"),
        AccountPositionType::Deposit,
        req.asset,
        market_update.market_index.supply_index.raw(),
        market_update.credited_amount,
        &position,
        None,
    );

    // Storage is written once after the whole supply batch completes.
    update_or_remove_supply_position(account, req.asset, &position);

    position
}

struct SupplyMarketUpdate {
    market_index: MarketIndex,
    credited_amount: i128,
}

fn update_market_position(
    env: &Env,
    cache: &mut ControllerCache,
    position: &mut AccountPosition,
    asset: &Address,
    amount: i128,
    asset_config: &AssetConfig,
    caller: &Address,
) -> SupplyMarketUpdate {
    let pool_addr = cache.cached_pool_address(asset);

    let credited_amount = pull_supply_tokens(env, caller, asset, &pool_addr, amount);

    apply_pool_supply(
        env,
        cache,
        asset,
        position,
        credited_amount,
        asset_config.supply_cap,
    )
}

fn pull_supply_tokens(
    env: &Env,
    caller: &Address,
    asset: &Address,
    pool_addr: &Address,
    amount: i128,
) -> i128 {
    let received = utils::transfer_and_measure_received(
        env,
        asset,
        caller,
        pool_addr,
        amount,
        GenericError::AmountMustBePositive,
    );

    validate_supply_credit(env, amount, received);

    received
}

fn validate_supply_credit(env: &Env, sent: i128, received: i128) {
    assert_with_error!(env, received > 0, GenericError::AmountMustBePositive);
    // Fee-on-transfer tokens may credit less than the requested amount.
    require_credit_not_above_sent(env, sent, received);
}

fn apply_pool_supply(
    env: &Env,
    cache: &mut ControllerCache,
    asset: &Address,
    position: &mut AccountPosition,
    amount: i128,
    supply_cap: i128,
) -> SupplyMarketUpdate {
    let pool_addr = cache.cached_pool_address(asset);
    let result = pool_supply_call(env, &pool_addr, (&*position).into(), amount, supply_cap);

    // Merge ONLY the scaled share back; the pool does not echo collateral risk
    // params, so preserve the ones the controller already holds.
    position.scaled_amount = Ray::from_raw(result.position.scaled_amount_ray);
    cache.record_market_update(&result.market_state);

    SupplyMarketUpdate {
        market_index: (&result.market_index).into(),
        credited_amount: amount,
    }
}

fn validate_bulk_isolation(
    env: &Env,
    account: &Account,
    assets: &Vec<Payment>,
    cache: &mut ControllerCache,
) {
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
    cache: &mut ControllerCache,
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
