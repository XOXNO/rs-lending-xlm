//! Blend V2 migration into controller positions.

use crate::account;
use common::errors::GenericError;
use common::types::{Account, DebtPosition, HubAssetKey, PositionMode};
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, symbol_short, token, Address, Env, IntoVal,
    Map, Vec,
};
use stellar_macros::when_not_paused;

use crate::context::Cache;
use crate::events::{self, BlendMigrationEvent};
use crate::external::blend::{
    blend_submit_call, BlendRequest, REQ_REPAY, REQ_WITHDRAW, REQ_WITHDRAW_COLLATERAL,
};
use crate::positions::supply;
use crate::strategies::swap::balance_delta;
use crate::strategies::{
    borrow_for_migration, prefetch_strategy_oracles, repay_debt_from_controller, strategy_finalize,
    StrategyRepay,
};
use crate::{risk::validation, storage, Controller, ControllerArgs, ControllerClient};

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

#[contractimpl]
impl Controller {
    /// Migrates a Blend V2 position into the controller.
    /// Debt caps bound the zero-fee borrow used to clear Blend debt.
    #[when_not_paused]
    pub fn migrate_from_blend(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        hub_id: u32,
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
                spoke_id,
                hub_id,
                blend_pool,
                collateral_assets,
                supply_assets,
                debt_caps,
            },
        )
    }
}

/// Migrate Blend V2 → controller: clear Blend debt, sweep assets, open positions.
///
/// Checklist: auth → reentrancy → preflight → account → markets → debt leg →
/// withdraw/deposit leg → finalize → event.
pub(crate) fn process_migrate_blend(env: &Env, caller: &Address, params: MigrateBlendParams) -> u64 {
    // 1–2. Auth + reentrancy
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

    // 3. Preflight (hub, non-empty request, approved Blend pool)
    validation::require_hub_active(env, hub_id);
    validate_migration_request(
        env,
        &blend_pool,
        &collateral_assets,
        &supply_assets,
        &debt_caps,
    );

    // 4–5. Account + oracles (deduped withdraw list)
    let (account_id, mut account, mut cache, withdraw_assets) = prepare_migration_account(
        env,
        caller,
        account_id,
        spoke_id,
        &collateral_assets,
        &supply_assets,
        &debt_caps,
    );

    // 3b. Markets active before any balance read or Blend call
    require_migration_markets_active(env, &mut cache, hub_id, &withdraw_assets, &debt_caps);

    // 6a. Zero-fee borrow → repay Blend debt → reconcile refunds
    execute_migration_debt_leg(
        env,
        caller,
        &blend_pool,
        hub_id,
        &debt_caps,
        &mut account,
        &mut cache,
    );

    // 6b. Sweep Blend collateral/supply → controller deposit
    if !withdraw_assets.is_empty() {
        let before_withdraw = snapshot_balances(env, &withdraw_assets);
        let withdraw_requests = build_withdraw_requests(env, &collateral_assets, &supply_assets);
        guarded_submit(env, &blend_pool, caller, &withdraw_requests);
        deposit_withdrawn(
            env,
            &mut account,
            &mut cache,
            hub_id,
            &withdraw_assets,
            &before_withdraw,
        );
    }

    // 7. Finalize + migration event
    strategy_finalize(env, account_id, &account, &mut cache);

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
    // Borrow before submit so post-submit delta only Blend's over-repay refund.
    let before_debt = snapshot_balances(env, &debt_asset_list(env, debt_caps));
    for (debt_asset, max) in debt_caps.iter() {
        validation::require_positive_amount(env, max);
        let hub_debt = HubAssetKey {
            hub_id,
            asset: debt_asset,
        };
        borrow_for_migration(env, account, &hub_debt, max, cache);
    }
    let repay_requests = build_repay_requests(env, debt_caps);
    authorize_repay_pulls(env, blend_pool, debt_caps);
    guarded_submit(env, blend_pool, caller, &repay_requests);
    reconcile_debt_refunds(env, account, cache, caller, hub_id, debt_caps, &before_debt);
}

/// Loads or creates the account under the migration guard, prefetches strategy
/// oracles, and returns it with the deduped withdraw-asset list.
fn prepare_migration_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    spoke_id: u32,
    collateral_assets: &Vec<Address>,
    supply_assets: &Vec<Address>,
    debt_caps: &Vec<(Address, i128)>,
) -> (u64, Account, Cache, Vec<Address>) {
    // Debt-opening flow: prices must be risk-increasing.
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
    prefetch_strategy_oracles(&mut cache, &account, &all_assets);
    (account_id, account, cache, withdraw_assets)
}

/// Requires every migration asset (deduped collateral ∪ supply, and each debt
/// asset) to be a configured market before any `.balance()` read or Blend call.
/// `require_market_active` checks the token-rooted oracle, so `hub_id` only names
/// the coordinate the positions open on.
fn require_migration_markets_active(
    env: &Env,
    cache: &mut Cache,
    hub_id: u32,
    withdraw_assets: &Vec<Address>,
    debt_caps: &Vec<(Address, i128)>,
) {
    for asset in withdraw_assets.iter() {
        validation::require_market_active(env, cache, &HubAssetKey { hub_id, asset });
    }
    for (asset, _) in debt_caps.iter() {
        validation::require_market_active(env, cache, &HubAssetKey { hub_id, asset });
    }
}

/// Rejects empty requests and unapproved Blend pools.
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
    // Only governance-approved Blend pool may be a migration source. This closes arbitrary external calls.
    assert_with_error!(
        env,
        storage::is_blend_pool_approved(env, blend_pool),
        GenericError::BlendPoolNotApproved
    );
}

/// Rejects duplicate debt assets and returns the deduped withdraw list plus the full asset set.
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
        let bal = token::Client::new(env, &asset).balance(&controller);
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

/// Runs Blend `submit` under the reentrancy guard. Callers that need Blend to
/// pull controller-held tokens must set up that authorization immediately
/// before this call (see `authorize_repay_pulls`); withdraw-only submits need
/// no such authorization.
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

/// Loads the account's debt position for `hub_debt`, trapping if absent.
fn load_debt_position(env: &Env, account: &Account, hub_debt: &HubAssetKey) -> DebtPosition {
    let raw = account
        .borrow_positions
        .get(hub_debt.clone())
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    DebtPosition::from(&raw)
}
