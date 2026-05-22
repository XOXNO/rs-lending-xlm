use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::math::fp::{Ray, Wad};
use common::types::{
    Account, AccountPosition, AccountPositionType, AssetConfig, EModeCategory, MarketIndex,
    Payment, PositionMode, PriceFeed,
};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Vec};
use stellar_macros::{only_role, when_not_paused};

use super::dust::require_no_dust_after;
use super::{emode, update};
use crate::cache::ControllerCache;
use crate::cross_contract::pool::pool_supply_call;
use crate::oracle::policy::OraclePolicy;
use crate::{helpers, storage, utils, validation, Controller, ControllerArgs, ControllerClient};

const THRESHOLD_UPDATE_MIN_HF_RAW: i128 = 1_050_000_000_000_000_000;

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

    #[when_not_paused]
    #[only_role(caller, "KEEPER")]
    pub fn update_account_threshold(
        env: Env,
        caller: Address,
        asset: Address,
        has_risks: bool,
        account_ids: Vec<u64>,
    ) {
        validation::require_not_flash_loaning(&env);

        // Propagates threshold updates with safety buffer.
        let mut cache = ControllerCache::new(&env, OraclePolicy::RiskIncreasing);
        validation::require_asset_supported(&env, &mut cache, &asset);

        let base_config = cache.cached_asset_config(&asset);
        let price_feed = cache.cached_price(&asset);

        for account_id in account_ids {
            let mut account_asset_config = base_config.clone();

            update_position_threshold(
                &env,
                account_id,
                ThresholdUpdate {
                    asset: &asset,
                    has_risks,
                    asset_config: &mut account_asset_config,
                    feed: &price_feed,
                },
                &mut cache,
            );
        }
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
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let (acct_id, mut account) =
        resolve_supply_account(env, caller, account_id, e_mode_category, assets);

    // Third-party deposits permitted.

    let mut cache = ControllerCache::new(env, OraclePolicy::RiskDecreasing);

    process_deposit(env, caller, acct_id, &mut account, assets, &mut cache);

    // Rejects dust on first-time supply.
    require_no_dust_after(env, &mut cache, &account);

    // Mutates supply positions only.
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
        let supply_positions = storage::get_positions(env, account_id, AccountPositionType::Deposit);
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

    prepare_deposit_plan(env, account, &deposit_plan, cache, &e_mode);
    execute_deposit_plan(
        env,
        caller,
        account_id,
        account,
        &deposit_plan,
        cache,
        &e_mode,
    );
}

fn prepare_deposit_plan(
    env: &Env,
    account: &Account,
    assets: &Vec<Payment>,
    cache: &mut ControllerCache,
    e_mode: &Option<EModeCategory>,
) {
    validation::validate_bulk_position_limits(env, account, AccountPositionType::Deposit, assets);
    validate_bulk_isolation(env, account, assets, cache);

    // Caps verified post-transfer.
    for (asset, _) in assets {
        validation::require_market_active(env, cache, &asset);

        let asset_config = emode::effective_asset_config(env, account, &asset, cache, e_mode);

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
    e_mode: &Option<EModeCategory>,
) {
    let _ = account_id;
    for (asset, amount_in) in assets {
        let asset_config = emode::effective_asset_config(env, account, &asset, cache, e_mode);

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

fn get_or_create_deposit_position(
    account: &Account,
    asset_config: &AssetConfig,
    asset: &Address,
) -> AccountPosition {
    account
        .supply_positions
        .get(asset.clone())
        .map(|raw| AccountPosition::from(&raw))
        .unwrap_or(AccountPosition {
            scaled_amount: Ray::ZERO,
            liquidation_threshold: asset_config.liquidation_threshold,
            liquidation_bonus: asset_config.liquidation_bonus,
            liquidation_fees: asset_config.liquidation_fees,
            loan_to_value: asset_config.loan_to_value,
        })
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
    let mut position = get_or_create_deposit_position(account, req.asset_config, req.asset);

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
    let result = pool_supply_call(env, &pool_addr, position.clone(), amount, supply_cap);

    *position = (&result.position).into();
    cache.record_market_update(&result.market_state);

    SupplyMarketUpdate {
        market_index: (&result.market_index).into(),
        credited_amount: amount,
    }
}

/// Per-account inputs for a keeper threshold propagation.
pub struct ThresholdUpdate<'a> {
    pub asset: &'a Address,
    pub has_risks: bool,
    pub asset_config: &'a mut AssetConfig,
    pub feed: &'a PriceFeed,
}

// Keeper-driven risk parameter propagation.
pub fn update_position_threshold(
    env: &Env,
    account_id: u64,
    update_req: ThresholdUpdate<'_>,
    cache: &mut ControllerCache,
) {
    let ThresholdUpdate {
        asset,
        has_risks,
        asset_config,
        feed,
    } = update_req;

    // No-op when the account is gone (bad-debt cleanup, full exit).
    let Some(meta) = storage::try_get_account_meta(env, account_id) else {
        return;
    };

    let supply_positions = storage::get_positions(env, account_id, AccountPositionType::Deposit);

    // No-op when the account has no supply position for this asset.
    let Some(position) = supply_positions.get(asset.clone()) else {
        return;
    };

    // Load borrow positions only when the health-factor gate requires them.
    let borrow_positions = if has_risks {
        storage::get_positions(env, account_id, AccountPositionType::Borrow)
    } else {
        soroban_sdk::Map::new(env)
    };

    storage::renew_user_account(env, account_id);

    // Apply e-mode overrides.
    let e_mode_category = emode::e_mode_category(env, meta.e_mode_category_id);
    let asset_emode_config = cache.cached_emode_asset(meta.e_mode_category_id, asset);
    emode::apply_e_mode_to_asset_config(env, asset_config, &e_mode_category, asset_emode_config);

    let mut updated_pos = position;

    let cfg_lt = asset_config.liquidation_threshold.raw() as u32;
    let cfg_ltv = asset_config.loan_to_value.raw() as u32;
    let cfg_bonus = asset_config.liquidation_bonus.raw() as u32;
    let cfg_fees = asset_config.liquidation_fees.raw() as u32;
    if has_risks {
        if updated_pos.liquidation_threshold_bps != cfg_lt {
            updated_pos.liquidation_threshold_bps = cfg_lt;
        }
    } else {
        if updated_pos.loan_to_value_bps != cfg_ltv {
            updated_pos.loan_to_value_bps = cfg_ltv;
        }
        if updated_pos.liquidation_bonus_bps != cfg_bonus {
            updated_pos.liquidation_bonus_bps = cfg_bonus;
        }
        if updated_pos.liquidation_fees_bps != cfg_fees {
            updated_pos.liquidation_fees_bps = cfg_fees;
        }
    }

    let mut account = Account {
        owner: meta.owner.clone(),
        is_isolated: meta.is_isolated,
        e_mode_category_id: meta.e_mode_category_id,
        mode: meta.mode,
        isolated_asset: meta.isolated_asset.clone(),
        supply_positions,
        borrow_positions,
    };
    update::update_or_remove_position(
        &mut account,
        AccountPositionType::Deposit,
        asset,
        &AccountPosition::from(&updated_pos),
    );

    // Persist only the supply side; borrow stays as-is.
    storage::set_positions(
        env,
        account_id,
        AccountPositionType::Deposit,
        &account.supply_positions,
    );

    // Enforce safety buffer on risky updates.
    if has_risks {
        let hf = helpers::calculate_health_factor(
            env,
            cache,
            &account.supply_positions,
            &account.borrow_positions,
        );
        if hf < Wad::from_raw(THRESHOLD_UPDATE_MIN_HF_RAW) {
            panic_with_error!(env, CollateralError::HealthFactorTooLow);
        }
    }

    // Record a position update with amount = 0; no deposit or withdraw
    // occurred, only a parameter change.
    let market_index = cache.cached_market_index(asset);
    cache.record_position_update(
        symbol_short!("param_upd"),
        AccountPositionType::Deposit,
        asset,
        market_index.supply_index.raw(),
        0,
        &AccountPosition::from(&updated_pos),
        Some(feed.price.raw()),
    );
    cache.emit_position_batch(account_id, &account);
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
