//! Blend V2 to controller position migration.
//!
//! Moves a user's Blend collateral, supply, and debt into controller in one
//! transaction with a zero-fee strategy borrow. The flow clears Blend debt,
//! sweeps Blend balances, re-supplies collateral, and gates the final account
//! state with `strategy_finalize`. Looped same-asset positions use separate
//! repay and withdraw submits so balance deltas do not alias.

use common::errors::GenericError;
use controller_interface::types::{Account, DebtPosition, HubAssetKey, PositionMode};
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, symbol_short, Address, Env, IntoVal, Map,
    Vec,
};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::events::{self, BlendMigrationEvent};
use crate::external::blend::{
    blend_submit_call, BlendRequest, REQ_REPAY, REQ_WITHDRAW, REQ_WITHDRAW_COLLATERAL,
};
use crate::positions::supply;
use crate::strategies::swap::balance_delta;
use crate::strategies::{
    open_migration_borrow, prefetch_strategy_oracles, repay_debt_from_controller,
    strategy_finalize, StrategyRepay,
};
use crate::{helpers, storage, validation, Controller, ControllerArgs, ControllerClient};

/// Parameters for `process_migrate_blend`.
pub struct MigrateBlendParams {
    pub account_id: u64,
    pub e_mode_category: u32,
    pub blend_pool: Address,
    pub collateral_assets: Vec<Address>,
    pub supply_assets: Vec<Address>,
    pub debt_caps: Vec<(Address, i128)>,
}

#[contractimpl]
impl Controller {
    /// Migrates a Blend V2 position into the controller.
    /// Debt caps bound the zero-fee borrow used to clear Blend debt.
    #[when_not_paused]
    pub fn migrate_from_blend(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        blend_pool: Address,
        collateral_assets: Vec<Address>,
        supply_assets: Vec<Address>,
        debt_caps: Vec<(Address, i128)>,
    ) -> u64 {
        process_migrate_blend(
            &env,
            &caller,
            MigrateBlendParams {
                account_id,
                e_mode_category,
                blend_pool,
                collateral_assets,
                supply_assets,
                debt_caps,
            },
        )
    }
}

pub fn process_migrate_blend(env: &Env, caller: &Address, params: MigrateBlendParams) -> u64 {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let MigrateBlendParams {
        account_id,
        e_mode_category,
        blend_pool,
        collateral_assets,
        supply_assets,
        debt_caps,
    } = params;

    assert_with_error!(
        env,
        !collateral_assets.is_empty() || !supply_assets.is_empty() || !debt_caps.is_empty(),
        GenericError::InvalidPayments
    );
    // Only a governance-approved Blend pool may be the migration source. This
    // closes the arbitrary-external-call / fee-free-flash-loan surface: an
    // attacker cannot substitute a contract they control as `blend_pool`.
    assert_with_error!(
        env,
        storage::is_blend_pool_approved(env, &blend_pool),
        GenericError::BlendPoolNotApproved
    );

    // Debt-opening flow: prices must be risk-increasing.
    let mut cache = Cache::new(env);

    let (account_id, mut account) = helpers::load_or_create_account(
        env,
        caller,
        account_id,
        e_mode_category,
        PositionMode::Normal,
        helpers::AccountGuard::Migrate,
        &mut cache,
    );

    // Reject duplicate debt entries (a duplicate would double-borrow and
    // double-repay the same asset).
    require_unique_debt_assets(env, &debt_caps);
    // Same-asset debt and withdraw roles use separate snapshots so deltas do not alias.
    let withdraw_assets = unique_withdraw_assets(env, &collateral_assets, &supply_assets);

    let mut all_assets = withdraw_assets.clone();
    for (asset, _) in debt_caps.iter() {
        all_assets.push_back(asset);
    }
    prefetch_strategy_oracles(&mut cache, &account, &all_assets);

    // Borrow before submit so the post-submit delta is only Blend's
    // over-repay refund.
    if !debt_caps.is_empty() {
        // D{debt_asset.decimals}{Token(debt_asset)} snapshots isolate borrow/repay/refund deltas.
        let before_debt = snapshot_balances(env, &debt_asset_list(env, &debt_caps));
        for (debt_asset, max) in debt_caps.iter() {
            validation::require_positive_amount(env, max);
            // D{debt_asset.decimals}{Token(debt_asset)} zero-fee cap equals received delta used to clear Blend debt.
            open_migration_borrow(env, &mut cache, &mut account, &debt_asset, max);
        }
        let repay_requests = build_repay_requests(env, &debt_caps);
        authorize_repay_pulls(env, &blend_pool, &debt_caps);
        guarded_submit(env, &blend_pool, caller, &repay_requests);
        reconcile_debt_refunds(
            env,
            &mut account,
            &mut cache,
            caller,
            &debt_caps,
            &before_debt,
        );
    }

    // Sweep Blend collateral and supply, then re-supply controller collateral.
    if !withdraw_assets.is_empty() {
        // D{asset.decimals}{Token(asset)} snapshots measure received collateral/supply sweeps.
        let before_withdraw = snapshot_balances(env, &withdraw_assets);
        let withdraw_requests = build_withdraw_requests(env, &collateral_assets, &supply_assets);
        // No controller-authed legs to pre-authorize: Blend's `submit` auth is
        // implicit (the controller is its direct invoker) and the withdrawals pay
        // the controller (authorized by Blend, the token spender).
        guarded_submit(env, &blend_pool, caller, &withdraw_requests);
        deposit_withdrawn(
            env,
            &mut account,
            &mut cache,
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

/// Rejects duplicate debt entries (a duplicate would double-borrow and
/// double-repay the same asset).
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
        let bal = soroban_sdk::token::Client::new(env, &asset).balance(&controller);
        before.set(asset, bal);
    }
    before
}

/// Repay requests, one per debt asset (`Repay(asset, cap)`).
fn build_repay_requests(env: &Env, debt_caps: &Vec<(Address, i128)>) -> Vec<BlendRequest> {
    let mut requests: Vec<BlendRequest> = Vec::new(env);
    for (asset, max) in debt_caps.iter() {
        requests.push_back(BlendRequest {
            request_type: REQ_REPAY,
            address: asset,
            amount: max,
        });
    }
    requests
}

/// Withdraw-all requests: collateral (`WithdrawCollateral`) then non-collateral
/// supply (`Withdraw`), each with `i128::MAX` to sweep the full balance.
fn build_withdraw_requests(
    env: &Env,
    collateral_assets: &Vec<Address>,
    supply_assets: &Vec<Address>,
) -> Vec<BlendRequest> {
    let mut requests: Vec<BlendRequest> = Vec::new(env);
    for asset in collateral_assets.iter() {
        requests.push_back(BlendRequest {
            request_type: REQ_WITHDRAW_COLLATERAL,
            address: asset,
            amount: i128::MAX,
        });
    }
    for asset in supply_assets.iter() {
        requests.push_back(BlendRequest {
            request_type: REQ_WITHDRAW,
            address: asset,
            amount: i128::MAX,
        });
    }
    requests
}

/// Runs Blend `submit` under the reentrancy guard.
/// Caller must authorize immediately before this call.
fn guarded_submit(env: &Env, blend_pool: &Address, from: &Address, requests: &Vec<BlendRequest>) {
    storage::with_flash_guard(env, || {
        let controller = env.current_contract_address();
        let _ = blend_submit_call(env, blend_pool, from, &controller, &controller, requests);
    });
}

/// Authorizes Blend debt-token pulls from the controller.
/// Withdraw-only submits have no debt caps and need no pull authorization.
fn authorize_repay_pulls(env: &Env, blend_pool: &Address, debt_caps: &Vec<(Address, i128)>) {
    if debt_caps.is_empty() {
        return;
    }
    let controller = env.current_contract_address();
    let mut entries: Vec<InvokerContractAuthEntry> = Vec::new(env);
    for (debt_asset, max) in debt_caps.iter() {
        entries.push_back(InvokerContractAuthEntry::Contract(SubContractInvocation {
            context: ContractContext {
                contract: debt_asset,
                fn_name: symbol_short!("transfer"),
                args: (controller.clone(), blend_pool.clone(), max).into_val(env),
            },
            sub_invocations: Vec::new(env),
        }));
    }
    env.authorize_as_current_contract(entries);
}

fn deposit_withdrawn(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    withdraw_assets: &Vec<Address>,
    before: &Map<Address, i128>,
) {
    let mut deposits: Vec<(HubAssetKey, i128)> = Vec::new(env);
    for asset in withdraw_assets.iter() {
        let token = soroban_sdk::token::Client::new(env, &asset);
        let prev = before.get(asset.clone()).unwrap_or(0);
        // D{asset.decimals}{Token(asset)} positive delta becomes controller supply deposit.
        let received = balance_delta(env, &token, prev);
        if received > 0 {
            deposits.push_back((HubAssetKey { hub_id: 0, asset }, received));
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

fn reconcile_debt_refunds(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    caller: &Address,
    debt_caps: &Vec<(Address, i128)>,
    before: &Map<Address, i128>,
) {
    for (debt_asset, _max) in debt_caps.iter() {
        let token = soroban_sdk::token::Client::new(env, &debt_asset);
        let prev = before.get(debt_asset.clone()).unwrap_or(0);
        // D{debt_asset.decimals}{Token(debt_asset)} Blend over-repay refund repays controller debt.
        let refund = balance_delta(env, &token, prev);
        if refund > 0 {
            let debt_pos = load_debt_position(env, account, &debt_asset);
            repay_debt_from_controller(
                env,
                account,
                cache,
                caller,
                StrategyRepay {
                    debt_token: &debt_asset,
                    debt_available: refund,
                    debt_pos: &debt_pos,
                    action: events::PositionAction::Migrate,
                },
            );
        }
    }
}

fn load_debt_position(env: &Env, account: &Account, debt_asset: &Address) -> DebtPosition {
    let raw = account
        .borrow_positions
        .get(HubAssetKey {
            hub_id: 0,
            asset: debt_asset.clone(),
        })
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    DebtPosition::from(&raw)
}
