use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::types::{
    Account, AccountPosition, AccountPositionType, AssetConfig, AssetConfigRaw, MarketIndex,
    Payment, PositionMode,
};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Map, Vec};
use stellar_macros::when_not_paused;

use super::dust::require_no_dust_after;
use super::{emode, update};
use crate::cache::ControllerCache;
use crate::cross_contract::pool::pool_supply_call;
use crate::oracle::policy::OraclePolicy;
use crate::{storage, utils, validation, Controller, ControllerArgs, ControllerClient};

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

// Processes supply batch.
pub fn process_supply(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    assets: &Vec<Payment>,
) -> u64 {
    // Stage 1: Pipelined Context Check
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    // Stage 2: State Resolution
    let (acct_id, mut account) =
        resolve_supply_account(env, caller, account_id, e_mode_category, assets);
    let mut cache = ControllerCache::new(env, OraclePolicy::RiskDecreasing);

    // Stage 3 & 4: Pre-flight Validation & Core Pool Execution
    process_deposit(env, caller, acct_id, &mut account, assets, &mut cache);

    // Stage 5: Post-flight Risk Gates
    // Rejects dust on first-time supply.
    require_no_dust_after(env, &mut cache, &account);

    // Stage 6: State Persistence
    storage::set_positions(
        env,
        acct_id,
        AccountPositionType::Deposit,
        &account.supply_positions,
    );
    cache.emit_position_batch(acct_id, &account);
    cache.emit_market_batch();

    acct_id
}

// Resolves account ID and returns snapshot.
fn resolve_supply_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    assets: &Vec<Payment>,
) -> (u64, Account) {
    validation::require_non_empty_payments(env, assets);

    if account_id == 0 {
        create_account_for_first_asset(env, caller, e_mode_category, assets)
    } else {
        let meta = storage::get_account_meta(env, account_id);
        let supply_positions =
            storage::get_positions(env, account_id, AccountPositionType::Deposit);
        let account =
            storage::account_from_parts(meta, supply_positions, soroban_sdk::Map::new(env));
        (account_id, account)
    }
}

// Processes deposit batch on account.
pub fn process_deposit(
    env: &Env,
    caller: &Address,
    account_id: u64,
    account: &mut Account,
    assets: &Vec<Payment>,
    cache: &mut ControllerCache,
) {
    // Fetch the e-mode category once and reuse across every iteration.
    let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);

    let deposit_plan = utils::aggregate_positive_payments(env, assets);

    // Resolve effective asset configs once; both prepare and execute read them.
    let mut effective_configs: Map<Address, AssetConfigRaw> = Map::new(env);
    for (asset, _) in deposit_plan.iter() {
        if !effective_configs.contains_key(asset.clone()) {
            let cfg = emode::effective_asset_config(env, account, &asset, cache, &e_mode);
            effective_configs.set(asset, (&cfg).into());
        }
    }

    prepare_deposit_plan(env, account, &deposit_plan, cache, &effective_configs);
    execute_deposit_plan(
        env,
        caller,
        account_id,
        account,
        &deposit_plan,
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
    validation::validate_bulk_position_limits(env, account, AccountPositionType::Deposit, assets);
    validate_bulk_isolation(env, account, assets, cache);

    // Caps verified post-transfer.
    for (asset, _) in assets {
        validation::require_market_active(env, cache, &asset);

        let asset_config: AssetConfig =
            (&validation::expect_invariant(env, effective_configs.get(asset.clone()))).into();

        emode::validate_e_mode_asset(env, cache, account.e_mode_category_id, &asset, true);
        emode::ensure_e_mode_compatible_with_asset(env, &asset_config, account.e_mode_category_id);

        if !asset_config.can_supply() {
            panic_with_error!(env, CollateralError::NotCollateral);
        }

        emode::validate_isolated_collateral(env, account, &asset, &asset_config);
    }
}

fn execute_deposit_plan(
    env: &Env,
    caller: &Address,
    account_id: u64,
    account: &mut Account,
    assets: &Vec<Payment>,
    cache: &mut ControllerCache,
    effective_configs: &Map<Address, AssetConfigRaw>,
) {
    let _ = account_id;
    for (asset, amount_in) in assets {
        let asset_config: AssetConfig =
            (&validation::expect_invariant(env, effective_configs.get(asset.clone()))).into();

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

/// Per-call deposit inputs.
pub struct DepositRequest<'a> {
    pub asset: &'a Address,
    pub amount: i128,
    pub asset_config: &'a AssetConfig,
}

// Updates deposit position and calls pool.
pub fn update_deposit_position(
    env: &Env,
    account: &mut Account,
    req: DepositRequest<'_>,
    caller: &Address,
    cache: &mut ControllerCache,
) -> AccountPosition {
    let mut position =
        account.get_or_create_position(AccountPositionType::Deposit, req.asset, req.asset_config);

    // Threshold only updated via keeper path.
    if position.loan_to_value != req.asset_config.loan_to_value {
        position.loan_to_value = req.asset_config.loan_to_value;
    }
    if position.liquidation_bonus != req.asset_config.liquidation_bonus {
        position.liquidation_bonus = req.asset_config.liquidation_bonus;
    }
    if position.liquidation_fees != req.asset_config.liquidation_fees {
        position.liquidation_fees = req.asset_config.liquidation_fees;
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

    // Update the in-memory account. `process_supply` writes storage once at
    // the end of the batch.
    update::update_or_remove_position(account, AccountPositionType::Deposit, req.asset, &position);

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
    if received <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }
    // Fee-on-transfer tokens may credit less.
    validation::require_credit_not_above_sent(env, sent, received);
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
    let result = pool_supply_call(env, &pool_addr, *position, amount, supply_cap);

    *position = (&result.position).into();
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
    let (first_asset, _) = validation::expect_invariant(env, assets.get(0));
    let first_config = cache.cached_asset_config(&first_asset);
    if account.is_isolated || first_config.is_isolated_asset {
        panic_with_error!(env, FlashLoanError::BulkSupplyNoIso);
    }
}

// Creates account for supply entry point.
fn create_account_for_first_asset(
    env: &Env,
    caller: &Address,
    e_mode_category: u32,
    assets: &Vec<Payment>,
) -> (u64, Account) {
    let (first_asset, _) = validation::expect_invariant(env, assets.get(0));
    let first_config = storage::get_market_config(env, &first_asset).asset_config;
    let is_isolated = first_config.is_isolated_asset;
    let isolated_asset = if is_isolated {
        Some(first_asset.clone())
    } else {
        None
    };
    super::account::create_account(
        env,
        caller,
        e_mode_category,
        PositionMode::Normal,
        is_isolated,
        isolated_asset,
    )
}
