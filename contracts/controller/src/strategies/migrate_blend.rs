//! Blend V2 → controller one-click position migration.
//!
//! Atomically moves a user's Blend position (collateral, non-collateral supply,
//! and debt) into the controller in a single transaction at zero flash-loan fee.
//!
//! The "flash loan" is a fee=0 `create_strategy` borrow (`open_migration_borrow`)
//! whose proceeds repay the user's Blend debt; Blend's collateral and supply are
//! withdrawn to the controller and re-supplied into the user's account; a single
//! end-state gate (`strategy_finalize`) enforces solvency. The Blend over-repay
//! refund is reconciled back into the new debt so the user's debt equals exactly
//! what cleared Blend.
//!
//! Authorization: the user authorizes this entrypoint and, in the transaction's
//! auth tree, Blend's `submit(from = user, ...)`; the controller authorizes its
//! own `spender` legs (the submit and the repay token pulls) via
//! `authorize_as_current_contract`, emitted immediately before the submit.
//!
//! See docs/superpowers/specs/2026-06-19-blend-v2-migration-design.md.

use common::errors::GenericError;
use controller_interface::types::{Account, DebtPosition, PositionMode};
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
use crate::oracle::policy::OraclePolicy;
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
    /// Migrates a Blend V2 position into the controller. `account_id == 0`
    /// creates a fresh account. `collateral_assets`/`supply_assets` are swept
    /// with "withdraw all" semantics; each `(debt_asset, max)` in `debt_caps`
    /// bounds the zero-fee borrow used to clear that Blend debt.
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
    let mut cache = Cache::new(env, OraclePolicy::RiskIncreasing);

    let (account_id, mut account) =
        load_or_create_migration_account(env, caller, account_id, e_mode_category);

    // Unique withdraw assets (collateral ∪ supply). Reject an asset that is both
    // withdrawn and borrowed: Blend would net the controller's transfers and the
    // repay-pull authorization amount would no longer match.
    let debt_set = debt_asset_set(env, &debt_caps);
    let withdraw_assets =
        unique_withdraw_assets(env, &collateral_assets, &supply_assets, &debt_set);

    let mut all_assets = withdraw_assets.clone();
    for (asset, _) in debt_caps.iter() {
        all_assets.push_back(asset);
    }
    prefetch_strategy_oracles(&mut cache, &account, &all_assets);

    // Snapshot controller balances BEFORE borrowing so post-submit deltas mean:
    // withdraw assets -> amount received; debt assets -> Blend over-repay refund.
    let before = snapshot_balances(env, &withdraw_assets, &debt_caps);

    // Open the zero-fee migration borrow for each debt asset (the "flash loan").
    for (debt_asset, max) in debt_caps.iter() {
        validation::require_positive_amount(env, max);
        open_migration_borrow(env, &mut cache, &mut account, &debt_asset, max);
    }

    // One Blend submit: repay all debt, then withdraw all collateral and supply
    // to the controller. Repaying first leaves the user debt-free before the
    // withdrawals, so Blend skips its post-action health check.
    let requests = build_blend_requests(env, &debt_caps, &collateral_assets, &supply_assets);
    authorize_blend_submit(env, &blend_pool, caller, &requests, &debt_caps);
    let controller = env.current_contract_address();
    // Re-entrancy guard around the external Blend call, mirroring the aggregator
    // swap path. The Soroban host already prohibits re-entering the controller;
    // this is defense-in-depth and keeps the strategy paths consistent.
    let guard_was_set = storage::is_flash_loan_ongoing(env);
    storage::set_flash_loan_ongoing(env, true);
    let _ = blend_submit_call(
        env,
        &blend_pool,
        caller,
        &controller,
        &controller,
        &requests,
    );
    if !guard_was_set {
        storage::set_flash_loan_ongoing(env, false);
    }

    // Re-supply everything withdrawn into the user's account (funds from controller).
    deposit_withdrawn(env, &mut account, &mut cache, &withdraw_assets, &before);

    // Net each new debt down to exactly what cleared Blend by repaying the refund.
    reconcile_debt_refunds(env, &mut account, &mut cache, caller, &debt_caps, &before);

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

fn load_or_create_migration_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
) -> (u64, Account) {
    if account_id == 0 {
        return helpers::create_account(env, caller, e_mode_category, PositionMode::Normal);
    }
    let account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);
    // Reject a conflicting non-zero e-mode arg for an existing account, matching
    // supply()'s EModeMismatch guard (the stored category always governs).
    if e_mode_category != 0 && e_mode_category != account.e_mode_category_id {
        panic_with_error!(env, common::errors::EModeError::EModeMismatch);
    }
    (account_id, account)
}

/// Set of debt assets; rejects duplicate debt entries (a duplicate would
/// double-borrow and double-repay the same asset).
fn debt_asset_set(env: &Env, debt_caps: &Vec<(Address, i128)>) -> Map<Address, bool> {
    let mut set: Map<Address, bool> = Map::new(env);
    for (asset, _) in debt_caps.iter() {
        assert_with_error!(
            env,
            !set.contains_key(asset.clone()),
            GenericError::AssetsAreTheSame
        );
        set.set(asset, true);
    }
    set
}

/// Deduplicated `collateral ∪ supply`, asserting none is also a debt asset.
fn unique_withdraw_assets(
    env: &Env,
    collateral_assets: &Vec<Address>,
    supply_assets: &Vec<Address>,
    debt_set: &Map<Address, bool>,
) -> Vec<Address> {
    let mut seen: Map<Address, bool> = Map::new(env);
    let mut out: Vec<Address> = Vec::new(env);
    for asset in collateral_assets.iter().chain(supply_assets.iter()) {
        assert_with_error!(
            env,
            !debt_set.contains_key(asset.clone()),
            GenericError::AssetsAreTheSame
        );
        if !seen.contains_key(asset.clone()) {
            seen.set(asset.clone(), true);
            out.push_back(asset);
        }
    }
    out
}

fn snapshot_balances(
    env: &Env,
    withdraw_assets: &Vec<Address>,
    debt_caps: &Vec<(Address, i128)>,
) -> Map<Address, i128> {
    let controller = env.current_contract_address();
    let mut before: Map<Address, i128> = Map::new(env);
    for asset in withdraw_assets.iter() {
        let bal = soroban_sdk::token::Client::new(env, &asset).balance(&controller);
        before.set(asset, bal);
    }
    for (asset, _) in debt_caps.iter() {
        let bal = soroban_sdk::token::Client::new(env, &asset).balance(&controller);
        before.set(asset, bal);
    }
    before
}

fn build_blend_requests(
    env: &Env,
    debt_caps: &Vec<(Address, i128)>,
    collateral_assets: &Vec<Address>,
    supply_assets: &Vec<Address>,
) -> Vec<BlendRequest> {
    let mut requests: Vec<BlendRequest> = Vec::new(env);
    for (asset, max) in debt_caps.iter() {
        requests.push_back(BlendRequest {
            request_type: REQ_REPAY,
            address: asset,
            amount: max,
        });
    }
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

/// Authorizes, as the controller, the `spender` legs of the Blend submit: the
/// submit call itself and one `transfer(controller -> blend_pool, max)` per debt
/// asset (the repay pull). Must be the call immediately preceding the submit.
fn authorize_blend_submit(
    env: &Env,
    blend_pool: &Address,
    user: &Address,
    requests: &Vec<BlendRequest>,
    debt_caps: &Vec<(Address, i128)>,
) {
    let controller = env.current_contract_address();
    let mut sub: Vec<InvokerContractAuthEntry> = Vec::new(env);
    for (debt_asset, max) in debt_caps.iter() {
        sub.push_back(InvokerContractAuthEntry::Contract(SubContractInvocation {
            context: ContractContext {
                contract: debt_asset,
                fn_name: symbol_short!("transfer"),
                args: (controller.clone(), blend_pool.clone(), max).into_val(env),
            },
            sub_invocations: Vec::new(env),
        }));
    }
    let entry = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: blend_pool.clone(),
            fn_name: symbol_short!("submit"),
            args: (
                user.clone(),
                controller.clone(),
                controller.clone(),
                requests.clone(),
            )
                .into_val(env),
        },
        sub_invocations: sub,
    });
    env.authorize_as_current_contract(soroban_sdk::vec![env, entry]);
}

fn deposit_withdrawn(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    withdraw_assets: &Vec<Address>,
    before: &Map<Address, i128>,
) {
    let mut deposits: Vec<(Address, i128)> = Vec::new(env);
    for asset in withdraw_assets.iter() {
        let token = soroban_sdk::token::Client::new(env, &asset);
        let prev = before.get(asset.clone()).unwrap_or(0);
        let received = balance_delta(env, &token, prev);
        if received > 0 {
            deposits.push_back((asset, received));
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
        .get(debt_asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    DebtPosition::from(&raw)
}
