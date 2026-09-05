use crate::account;
use common::collections::push_unique_address;
use common::errors::GenericError;
use common::types::{Account, DebtPosition, HubAssetKey, PositionMode};
use common::validation::{expect_invariant, require_positive_amount};
use soroban_sdk::{assert_with_error, Address, Env, Map, Vec};

use crate::config;
use crate::context::Context;
use crate::events::{BlendMigrationEvent, PositionAction};
use crate::external::blend::{blend_repay_all, blend_sweep_all};
use crate::payments::{balance_delta_since, snapshot_balances};
use crate::positions::{require_can_supply, supply};
use crate::risk::validation::require_authorized_caller;
use crate::storage;
use crate::strategies::{
    borrow_into_controller, prefetch_strategy_prices, repay_debt_from_controller,
    strategy_finalize, StrategyRepay,
};

pub(crate) struct MigrateBlendParams {
    pub account_id: u64,
    pub spoke_id: u32,

    pub hub_id: u32,
    pub blend_pool: Address,
    pub collateral_assets: Vec<Address>,
    pub supply_assets: Vec<Address>,
    pub debt_caps: Vec<(Address, i128)>,
}

/// Replaces approved Blend debt with hub debt, then deposits measured Blend
/// withdrawals as hub supply. Returns the account id after risk checks.
pub(crate) fn process_migrate_blend(
    env: &Env,
    caller: &Address,
    params: MigrateBlendParams,
) -> u64 {
    require_authorized_caller(env, caller);

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

    require_unique_debt_assets(env, &debt_caps);

    let mut cache = Context::new(env);
    let (account_id, mut account) = account::load_or_create_account(
        env,
        caller,
        account_id,
        spoke_id,
        PositionMode::Normal,
        account::AccountGuard::Migrate,
        &mut cache,
    );

    // Both Blend balance types become hub supply; deduplicate their assets.
    let mut withdraw_assets: Vec<Address> = Vec::new(env);
    for asset in collateral_assets.iter().chain(supply_assets.iter()) {
        push_unique_address(&mut withdraw_assets, asset);
    }
    let mut price_assets = withdraw_assets.clone();
    for (asset, _) in debt_caps.iter() {
        price_assets.push_back(asset);
    }
    prefetch_strategy_prices(&mut cache, &account, &price_assets);

    // Check every supply destination before borrowing or moving Blend funds.
    for asset in withdraw_assets.iter() {
        let hub_asset = HubAssetKey { hub_id, asset };
        require_can_supply(env, &mut cache, spoke_id, &hub_asset);
    }

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
        let before_withdraw =
            snapshot_balances(env, &env.current_contract_address(), withdraw_assets.iter());
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

/// Borrows each debt cap, repays Blend in one call, then uses remaining funds
/// above the pre-borrow snapshot to reduce the new hub debt.
fn execute_migration_debt_leg(
    env: &Env,
    caller: &Address,
    blend_pool: &Address,
    hub_id: u32,
    debt_caps: &Vec<(Address, i128)>,
    account: &mut Account,
    cache: &mut Context,
) {
    if debt_caps.is_empty() {
        return;
    }

    let before_debt = snapshot_balances(
        env,
        &env.current_contract_address(),
        debt_caps.iter().map(|(asset, _)| asset),
    );
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

/// Requires a non-empty migration from an approved Blend pool.
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

/// Rejects repeated debt assets.
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

/// Deposits only positive controller receipts since the pre-withdraw snapshot.
fn deposit_withdrawn(
    env: &Env,
    account: &mut Account,
    cache: &mut Context,
    hub_id: u32,
    withdraw_assets: &Vec<Address>,
    before: &Map<Address, i128>,
) {
    let mut deposits: Vec<(HubAssetKey, i128)> = Vec::new(env);
    let controller = env.current_contract_address();
    for asset in withdraw_assets.iter() {
        let prev = before.get(asset.clone()).unwrap_or(0);

        let received = balance_delta_since(env, &asset, &controller, prev);
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

/// Repays hub debt using positive balances above the pre-borrow snapshot.
/// Pre-existing controller funds remain untouched.
fn reconcile_debt_refunds(
    env: &Env,
    account: &mut Account,
    cache: &mut Context,
    caller: &Address,
    hub_id: u32,
    debt_caps: &Vec<(Address, i128)>,
    before: &Map<Address, i128>,
) {
    let controller = env.current_contract_address();
    for (debt_asset, _max) in debt_caps.iter() {
        let prev = before.get(debt_asset.clone()).unwrap_or(0);

        let refund = balance_delta_since(env, &debt_asset, &controller, prev);
        if refund > 0 {
            let hub_debt = HubAssetKey {
                hub_id,
                asset: debt_asset.clone(),
            };
            let raw = expect_invariant(env, account.borrow_positions.get(hub_debt.clone()));
            let debt_pos = DebtPosition::from(&raw);
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
