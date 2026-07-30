//! Blend V2 → controller migration in one transaction.
//!
//! Caller auth; approved Blend pool only. Zero-fee strategy borrow clears Blend
//! debt; swept assets deposit as controller collateral. Finalizes with LTV/HF.

use crate::account;
use common::errors::{CollateralError, GenericError};
use common::types::{Account, DebtPosition, HubAssetKey, PositionMode};
use common::validation::require_positive_amount;
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env, Map, Vec};

use crate::config;
use crate::context::Cache;
use crate::events::{self, BlendMigrationEvent};
use crate::external::blend::{blend_repay_all, blend_sweep_all};
use crate::payments::balance_delta;
use crate::positions::{enforce_spoke_asset_flags, supply, FreezePolicy};
use crate::strategies::{
    borrow_for_migration, prefetch_strategy_prices, repay_debt_from_controller, strategy_finalize,
    StrategyRepay,
};
use crate::{risk::validation, storage};

pub(crate) struct MigrateBlendParams {
    pub account_id: u64,
    pub spoke_id: u32,
    /// Hub on which every controller-side position (debt and supply) is opened.
    pub hub_id: u32,
    pub blend_pool: Address,
    pub collateral_assets: Vec<Address>,
    pub supply_assets: Vec<Address>,
    pub debt_caps: Vec<(Address, i128)>,
}

/// Migrate Blend V2 → controller: clear Blend debt, sweep assets, open positions.
pub(crate) fn process_migrate_blend(
    env: &Env,
    caller: &Address,
    params: MigrateBlendParams,
) -> u64 {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let MigrateBlendParams {
        account_id,
        spoke_id,
        hub_id,
        blend_pool,
        collateral_assets,
        supply_assets,
        debt_caps,
    } = params;

    config::require_hub_active(env, hub_id);
    validate_migration_request(
        env,
        &blend_pool,
        &collateral_assets,
        &supply_assets,
        &debt_caps,
    );

    let (account_id, mut account, mut cache, withdraw_assets, all_assets) =
        prepare_migration_account(
            env,
            caller,
            account_id,
            spoke_id,
            &collateral_assets,
            &supply_assets,
            &debt_caps,
        );

    // Debt-opening flow: prices must be risk-increasing. Unconfigured assets
    // fail closed on the price-aggregator bulk read (`OracleNotConfigured`).
    prefetch_strategy_prices(&mut cache, &account, &all_assets);

    // Fail fast before any Blend call: a priced-but-unlisted (or non-supplyable)
    // withdraw asset would otherwise only be rejected by `process_deposit`
    // AFTER the external sweep. Debt assets are gated by the borrow entry gates
    // inside the debt leg, which also runs before the repay.
    require_withdraw_assets_supplyable(env, &mut cache, spoke_id, hub_id, &withdraw_assets);

    execute_migration_debt_leg(
        env,
        caller,
        &blend_pool,
        hub_id,
        &debt_caps,
        &mut account,
        &mut cache,
    );

    if !withdraw_assets.is_empty() {
        let before_withdraw = snapshot_balances(env, &withdraw_assets);
        blend_sweep_all(env, &blend_pool, caller, &collateral_assets, &supply_assets);
        deposit_withdrawn(
            env,
            &mut account,
            &mut cache,
            hub_id,
            &withdraw_assets,
            &before_withdraw,
        );
    }

    strategy_finalize(env, account_id, &mut account, &mut cache);

    BlendMigrationEvent {
        account_id,
        blend_pool,
        collateral_count: collateral_assets.len(),
        supply_count: supply_assets.len(),
        debt_count: debt_caps.len(),
    }
    .publish(env);

    account_id
}

/// Borrows and repays each Blend debt asset, reconciling Blend's over-repay refunds.
fn execute_migration_debt_leg(
    env: &Env,
    caller: &Address,
    blend_pool: &Address,
    hub_id: u32,
    debt_caps: &Vec<(Address, i128)>,
    account: &mut Account,
    cache: &mut Cache,
) {
    if debt_caps.is_empty() {
        return;
    }
    // Borrow before submit so post-submit delta is only Blend's over-repay refund.
    let before_debt = snapshot_balances(env, &debt_asset_list(env, debt_caps));
    for (debt_asset, max) in debt_caps.iter() {
        require_positive_amount(env, max);
        let hub_debt = HubAssetKey {
            hub_id,
            asset: debt_asset,
        };
        borrow_for_migration(env, account, &hub_debt, max, cache);
    }
    blend_repay_all(env, blend_pool, caller, debt_caps);
    reconcile_debt_refunds(env, account, cache, caller, hub_id, debt_caps, &before_debt);
}

fn prepare_migration_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    spoke_id: u32,
    collateral_assets: &Vec<Address>,
    supply_assets: &Vec<Address>,
    debt_caps: &Vec<(Address, i128)>,
) -> (u64, Account, Cache, Vec<Address>, Vec<Address>) {
    let mut cache = Cache::new(env);
    let (account_id, account) = account::load_or_create_account(
        env,
        caller,
        account_id,
        spoke_id,
        PositionMode::Normal,
        account::AccountGuard::Migrate,
        &mut cache,
    );
    let (withdraw_assets, all_assets) =
        prepare_migration_assets(env, collateral_assets, supply_assets, debt_caps);
    (account_id, account, cache, withdraw_assets, all_assets)
}

/// Every migrated withdraw asset must be listed, unpaused/unfrozen, and
/// supply-enabled on the destination spoke — the same gates
/// `validate_position_entry_gates` applies to the eventual deposit, pulled
/// forward so unsupported assets are rejected before any Blend interaction.
fn require_withdraw_assets_supplyable(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    hub_id: u32,
    withdraw_assets: &Vec<Address>,
) {
    for asset in withdraw_assets.iter() {
        let hub_asset = HubAssetKey { hub_id, asset };
        let asset_config = cache.require_listed_active_config(spoke_id, &hub_asset);
        enforce_spoke_asset_flags(env, cache, spoke_id, &hub_asset, FreezePolicy::BlockOnEntry);
        assert_with_error!(
            env,
            asset_config.can_supply(),
            CollateralError::NotCollateral
        );
    }
}

fn validate_migration_request(
    env: &Env,
    blend_pool: &Address,
    collateral_assets: &Vec<Address>,
    supply_assets: &Vec<Address>,
    debt_caps: &Vec<(Address, i128)>,
) {
    assert_with_error!(
        env,
        !collateral_assets.is_empty() || !supply_assets.is_empty() || !debt_caps.is_empty(),
        GenericError::InvalidPayments
    );
    // Allowlist: only governance-approved Blend pools.
    assert_with_error!(
        env,
        storage::is_blend_pool_approved(env, blend_pool),
        GenericError::BlendPoolNotApproved
    );
}

fn prepare_migration_assets(
    env: &Env,
    collateral_assets: &Vec<Address>,
    supply_assets: &Vec<Address>,
    debt_caps: &Vec<(Address, i128)>,
) -> (Vec<Address>, Vec<Address>) {
    require_unique_debt_assets(env, debt_caps);
    let withdraw_assets = unique_withdraw_assets(env, collateral_assets, supply_assets);
    let mut all_assets = withdraw_assets.clone();
    for (asset, _) in debt_caps.iter() {
        all_assets.push_back(asset);
    }
    (withdraw_assets, all_assets)
}

fn require_unique_debt_assets(env: &Env, debt_caps: &Vec<(Address, i128)>) {
    let mut seen: Map<Address, bool> = Map::new(env);
    for (asset, _) in debt_caps.iter() {
        assert_with_error!(
            env,
            !seen.contains_key(asset.clone()),
            GenericError::AssetsAreTheSame
        );
        seen.set(asset, true);
    }
}

/// The debt assets, in input order, as an address list (for snapshotting).
fn debt_asset_list(env: &Env, debt_caps: &Vec<(Address, i128)>) -> Vec<Address> {
    let mut out: Vec<Address> = Vec::new(env);
    for (asset, _) in debt_caps.iter() {
        out.push_back(asset);
    }
    out
}

/// Deduplicated `collateral ∪ supply`, preserving first-seen order.
fn unique_withdraw_assets(
    env: &Env,
    collateral_assets: &Vec<Address>,
    supply_assets: &Vec<Address>,
) -> Vec<Address> {
    let mut seen: Map<Address, bool> = Map::new(env);
    let mut out: Vec<Address> = Vec::new(env);
    for asset in collateral_assets.iter().chain(supply_assets.iter()) {
        if !seen.contains_key(asset.clone()) {
            seen.set(asset.clone(), true);
            out.push_back(asset);
        }
    }
    out
}

/// Snapshots the controller's token balance for each asset.
fn snapshot_balances(env: &Env, assets: &Vec<Address>) -> Map<Address, i128> {
    let controller = env.current_contract_address();
    let mut before: Map<Address, i128> = Map::new(env);
    for asset in assets.iter() {
        let bal = token::Client::new(env, &asset).balance(&controller);
        before.set(asset, bal);
    }
    before
}

/// Deposits the positive balance delta of each swept asset as controller collateral.
fn deposit_withdrawn(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    hub_id: u32,
    withdraw_assets: &Vec<Address>,
    before: &Map<Address, i128>,
) {
    let mut deposits: Vec<(HubAssetKey, i128)> = Vec::new(env);
    for asset in withdraw_assets.iter() {
        let token = token::Client::new(env, &asset);
        let prev = before.get(asset.clone()).unwrap_or(0);
        // D{asset.decimals}{Token(asset)} positive delta becomes controller supply deposit.
        let received = balance_delta(env, &token, prev);
        if received > 0 {
            // Migration opens controller positions on the caller-supplied `hub_id`;
            // the source asset list names Blend-side tokens, not hub coordinates.
            deposits.push_back((HubAssetKey { hub_id, asset }, received));
        }
    }
    if !deposits.is_empty() {
        supply::process_deposit(
            env,
            &env.current_contract_address(),
            account,
            &deposits,
            cache,
        );
    }
}

/// Repays controller debt with any Blend over-repay refund for each debt asset.
fn reconcile_debt_refunds(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    caller: &Address,
    hub_id: u32,
    debt_caps: &Vec<(Address, i128)>,
    before: &Map<Address, i128>,
) {
    for (debt_asset, _max) in debt_caps.iter() {
        let token = token::Client::new(env, &debt_asset);
        let prev = before.get(debt_asset.clone()).unwrap_or(0);
        // D{debt_asset.decimals}{Token(debt_asset)} Blend over-repay refund repays controller debt.
        let refund = balance_delta(env, &token, prev);
        if refund > 0 {
            let hub_debt = HubAssetKey {
                hub_id,
                asset: debt_asset.clone(),
            };
            let debt_pos = load_debt_position(env, account, &hub_debt);
            repay_debt_from_controller(
                env,
                account,
                cache,
                caller,
                StrategyRepay {
                    debt: &hub_debt,
                    debt_available: refund,
                    debt_pos: &debt_pos,
                    action: events::PositionAction::Migrate,
                },
            );
        }
    }
}

fn load_debt_position(env: &Env, account: &Account, hub_debt: &HubAssetKey) -> DebtPosition {
    let raw = account
        .borrow_positions
        .get(hub_debt.clone())
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    DebtPosition::from(&raw)
}
