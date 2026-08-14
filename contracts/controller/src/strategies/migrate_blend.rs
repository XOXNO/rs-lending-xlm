use crate::account;
use common::errors::GenericError;
use common::types::{Account, DebtPosition, HubAssetKey, PositionMode};
use common::validation::require_positive_amount;
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env, Map, Vec};

use crate::config;
use crate::context::Cache;
use crate::events::{BlendMigrationEvent, PositionAction};
use crate::external::blend::{blend_repay_all, blend_sweep_all};
use crate::positions::{require_can_supply, supply};
use crate::strategies::{
    borrow_into_controller, prefetch_strategy_prices, repay_debt_from_controller,
    strategy_finalize, StrategyRepay,
};
use crate::{risk::validation, storage};

pub(crate) struct MigrateBlendParams {
    pub account_id: u64,
    pub spoke_id: u32,

    pub hub_id: u32,
    pub blend_pool: Address,
    pub collateral_assets: Vec<Address>,
    pub supply_assets: Vec<Address>,
    pub debt_caps: Vec<(Address, i128)>,
}

/// Migrates a position out of an approved Blend pool into this hub for
/// `caller`: borrows `debt_caps` amounts into the controller to repay the
/// equivalent Blend debt, then sweeps `collateral_assets` and
/// `supply_assets` out of Blend and deposits what arrives as new hub supply
/// positions. Panics if `blend_pool` is not approved or the request carries
/// no assets. Finalizes with the standard solvency checks and returns the
/// account id.
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

    prefetch_strategy_prices(&mut cache, &account, &all_assets);

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

/// For each `(debt_asset, max)` in `debt_caps`, borrows `max` from the hub
/// into the controller as new debt for `account`, then repays that asset's
/// Blend debt. Any borrowed amount Blend did not consume is repaid straight
/// back against the new hub debt so `account` ends up owing only what Blend
/// actually needed. No-op when `debt_caps` is empty.
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

    let before_debt = snapshot_balances(env, &debt_asset_list(env, debt_caps));
    for (debt_asset, max) in debt_caps.iter() {
        require_positive_amount(env, max);
        let hub_debt = HubAssetKey {
            hub_id,
            asset: debt_asset,
        };
        borrow_into_controller(
            env,
            account,
            &hub_debt,
            max,
            false,
            PositionAction::Migrate,
            cache,
        );
    }
    blend_repay_all(env, blend_pool, caller, debt_caps);
    reconcile_debt_refunds(env, account, cache, caller, hub_id, debt_caps, &before_debt);
}

/// Loads or creates `account_id`'s account under the migrate guard
/// (owner/delegate and spoke checks) and builds the deduplicated
/// withdraw-asset list and combined asset list used for price prefetching.
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

/// Panics if any `withdraw_assets` is not listed and active as collateral in
/// `spoke_id`/`hub_id`, checked before any Blend funds are moved.
fn require_withdraw_assets_supplyable(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    hub_id: u32,
    withdraw_assets: &Vec<Address>,
) {
    for asset in withdraw_assets.iter() {
        let hub_asset = HubAssetKey { hub_id, asset };
        require_can_supply(env, cache, spoke_id, &hub_asset);
    }
}

/// Panics if the request carries no collateral, supply, or debt assets, or
/// if `blend_pool` is not on the approved list.
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

    assert_with_error!(
        env,
        storage::is_blend_pool_approved(env, blend_pool),
        GenericError::BlendPoolNotApproved
    );
}

/// Validates that `debt_caps` has no duplicate assets and returns the
/// deduplicated withdraw-asset list (`collateral_assets` union
/// `supply_assets`) alongside that list extended with the debt assets.
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

/// Panics with `AssetsAreTheSame` if `debt_caps` lists the same asset more
/// than once.
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

/// Extracts the asset addresses from `debt_caps`, dropping the per-asset
/// caps.
fn debt_asset_list(env: &Env, debt_caps: &Vec<(Address, i128)>) -> Vec<Address> {
    let mut out: Vec<Address> = Vec::new(env);
    for (asset, _) in debt_caps.iter() {
        out.push_back(asset);
    }
    out
}

/// Returns the deduplicated union of `collateral_assets` and
/// `supply_assets`, preserving first-seen order.
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

/// Records the controller's current balance for each of `assets`, keyed by
/// asset address.
fn snapshot_balances(env: &Env, assets: &Vec<Address>) -> Map<Address, i128> {
    let controller = env.current_contract_address();
    let mut before: Map<Address, i128> = Map::new(env);
    for asset in assets.iter() {
        let bal = token::Client::new(env, &asset).balance(&controller);
        before.set(asset, bal);
    }
    before
}

/// Deposits into `account`'s hub supply positions whatever balance of each
/// `withdraw_assets` arrived at the controller since the `before` snapshot;
/// no-op for assets with no increase.
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

        let received = token
            .balance(&env.current_contract_address())
            .checked_sub(prev)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
        if received > 0 {
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

/// Repays back into `account`'s hub debt position whatever balance of each
/// `debt_caps` asset accumulated at the controller since the `before`
/// snapshot, refunding the portion of the borrowed amount Blend did not
/// consume; no-op for assets with no increase.
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

        let refund = token
            .balance(&env.current_contract_address())
            .checked_sub(prev)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
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
                    action: PositionAction::Migrate,
                },
            );
        }
    }
}

/// Returns `account`'s debt position for `hub_debt`, panicking with
/// `InternalError` if none exists.
fn load_debt_position(env: &Env, account: &Account, hub_debt: &HubAssetKey) -> DebtPosition {
    let raw = account
        .borrow_positions
        .get(hub_debt.clone())
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    DebtPosition::from(&raw)
}
