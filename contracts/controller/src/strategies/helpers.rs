//! Shared strategy helpers for aggregator swaps and balance-delta checks.
//!
//! Strategies never trust router-reported amounts. The controller treats route
//! bytes as opaque, snapshots token balances before the router call, and verifies
//! input spend and output receipt from observed SAC balances.

use common::errors::GenericError;
use common::types::{Account, AccountPosition, AccountPositionType, DebtPosition, StrategySwap};
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    assert_with_error, panic_with_error, symbol_short, Address, Env, IntoVal, Symbol, Vec,
};

use crate::cache::Cache;
use crate::helpers::{require_no_borrow_dust_for_assets, require_no_supply_dust_for_assets};
use crate::positions::repay::{self, RepaymentRequest};
use crate::positions::withdraw::{self, WithdrawFlags, WithdrawalRequest, WITHDRAW_ALL_SENTINEL};
use crate::utils::EventContext;
use crate::{positions::borrow, storage, utils, validation};

pub(crate) mod aggregator {
    use soroban_sdk::{contractclient, Address, Bytes, Env};

    #[allow(dead_code)] // Generates the Soroban client proxy.
    #[contractclient(name = "AggregatorClient")]
    pub trait Aggregator {
        fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128;
    }
}

pub(crate) struct StrategyRepay<'a> {
    pub debt_token: &'a Address,
    pub debt_available: i128,
    pub debt_pos: &'a DebtPosition,
    pub action: Symbol,
}

pub(crate) struct StrategyWithdraw<'a> {
    pub asset: &'a Address,
    pub amount: i128,
    pub position: &'a AccountPosition,
    pub action: Symbol,
}

struct SwapBalanceSnapshot {
    pub token_in: i128,
    pub token_out: i128,
}

fn controller_event_context(env: &Env, action: Symbol) -> EventContext {
    EventContext {
        caller: env.current_contract_address(),
        action,
    }
}

pub(crate) fn open_strategy_borrow(
    env: &Env,
    cache: &mut Cache,
    account: &mut Account,
    asset: &Address,
    amount: i128,
) -> i128 {
    let new_borrow_assets = soroban_sdk::vec![env, (asset.clone(), amount)];
    validation::validate_bulk_position_limits(
        env,
        account,
        AccountPositionType::Borrow,
        &new_borrow_assets,
    );

    borrow::create_borrow_strategy(env, cache, account, asset, amount)
}

pub(crate) fn repay_debt_from_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    caller: &Address,
    req: StrategyRepay<'_>,
) {
    let debt_pool_addr = cache.cached_pool_address(req.debt_token);
    let debt_feed = cache.cached_price(req.debt_token);
    let debt_tok = soroban_sdk::token::Client::new(env, req.debt_token);

    // Listed debt tokens are 1:1 SACs (ADR-0006): the transfer is trusted, so
    // `debt_available` reaches the pool exactly — no balance-delta check needed.
    // The refund below does snapshot, since the aggregator is not trusted.
    utils::transfer_amount(
        env,
        req.debt_token,
        &env.current_contract_address(),
        &debt_pool_addr,
        req.debt_available,
        GenericError::InternalError,
    );

    let controller_balance_before_repay = debt_tok.balance(&env.current_contract_address());

    // Routes through the shared repay path for isolated-debt handling.
    repay::execute_repayment(
        env,
        account,
        controller_event_context(env, req.action),
        RepaymentRequest {
            asset: req.debt_token,
            position: req.debt_pos,
            amount: req.debt_available,
            price: debt_feed.price,
        },
        cache,
    );

    refund_controller_balance_delta(env, req.debt_token, controller_balance_before_repay, caller);
}

pub(crate) fn withdraw_collateral_to_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    req: StrategyWithdraw<'_>,
) -> i128 {
    let feed = cache.cached_price(req.asset);
    let token = soroban_sdk::token::Client::new(env, req.asset);
    let balance_before = token.balance(&env.current_contract_address());

    withdraw::execute_withdrawal(
        env,
        account,
        controller_event_context(env, req.action),
        WithdrawalRequest {
            asset: req.asset,
            amount: req.amount,
            position: req.position,
            price: feed.price,
        },
        WithdrawFlags::plain(),
        cache,
    );

    balance_delta(env, &token, balance_before)
}

// Any router-leftover input (underspend) stays on the controller;
// `verify_router_input_spend` bounds overspend, not underspend.
pub(crate) fn swap_tokens(
    env: &Env,
    token_in: &Address,
    amount_in: i128,
    token_out: &Address,
    swap: &StrategySwap,
) -> i128 {
    let router_addr = storage::get_aggregator(env);
    let router = aggregator::AggregatorClient::new(env, &router_addr);
    let token_out_client = soroban_sdk::token::Client::new(env, token_out);
    let token_in_client = soroban_sdk::token::Client::new(env, token_in);

    validate_strategy_swap(env, swap, amount_in);

    // Snapshot balances so `verify_router_output` can check the exact delta —
    // a defensive guard against future router ABI drift.
    let balance_before = snapshot_swap_balances(env, &token_in_client, &token_out_client);

    // Pre-authorize only the router's input-token pull; its other transfers
    // are router-initiated and covered by direct-caller attestation.
    pre_authorize_router_pull(env, &router_addr, token_in, amount_in);

    call_router_with_reentrancy_guard(env, &router, amount_in, swap);

    // Defense-in-depth: reject any pull above the committed input. Underspend
    // stays on the controller; the aggregator payload enforces slippage.
    verify_router_input_spend(env, &token_in_client, balance_before.token_in, amount_in);

    verify_router_output(env, &token_out_client, balance_before.token_out)
}

/// Rejects swap requests with impossible controller-side bounds.
fn validate_strategy_swap(env: &Env, swap: &StrategySwap, amount_in: i128) {
    if amount_in <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }
    assert_with_error!(env, !swap.is_empty(), GenericError::InvalidPayments);
}

fn snapshot_swap_balances(
    env: &Env,
    token_in_client: &soroban_sdk::token::Client,
    token_out_client: &soroban_sdk::token::Client,
) -> SwapBalanceSnapshot {
    SwapBalanceSnapshot {
        token_in: token_in_client.balance(&env.current_contract_address()),
        token_out: token_out_client.balance(&env.current_contract_address()),
    }
}

/// Invokes the router while preserving any outer flash-loan guard.
fn call_router_with_reentrancy_guard(
    env: &Env,
    router: &aggregator::AggregatorClient,
    amount_in: i128,
    swap: &StrategySwap,
) {
    let previously_set = storage::is_flash_loan_ongoing(env);
    storage::set_flash_loan_ongoing(env, true);
    let sender = env.current_contract_address();
    let _ = router.execute_strategy(&sender, &amount_in, swap);
    if !previously_set {
        storage::set_flash_loan_ongoing(env, false);
    }
}

/// Pre-authorizes the router to pull the strategy input token from the controller.
fn pre_authorize_router_pull(
    env: &Env,
    router_addr: &Address,
    token_in: &Address,
    amount_in: i128,
) {
    let entry = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: token_in.clone(),
            fn_name: symbol_short!("transfer"),
            args: (
                env.current_contract_address(),
                router_addr.clone(),
                amount_in,
            )
                .into_val(env),
        },
        sub_invocations: Vec::new(env),
    });
    env.authorize_as_current_contract(soroban_sdk::vec![env, entry]);
}

/// Rejects router input-token spend above the controller's committed amount.
fn verify_router_input_spend(
    env: &Env,
    token_in_client: &soroban_sdk::token::Client,
    balance_before: i128,
    amount_in: i128,
) {
    let balance_after = token_in_client.balance(&env.current_contract_address());
    assert_with_error!(
        env,
        balance_after <= balance_before,
        GenericError::InternalError
    );
    let actual_in_spent = balance_before - balance_after;
    // Allow underspend (leftover stays on the controller; the aggregator
    // payload enforces slippage); reject overspend — router/token pulled more
    // than committed.
    assert_with_error!(
        env,
        actual_in_spent <= amount_in,
        GenericError::InternalError
    );
}

fn verify_router_output(
    env: &Env,
    token_out_client: &soroban_sdk::token::Client,
    balance_before: i128,
) -> i128 {
    // Received must be non-negative; a decrease means aggregator/token misbehavior.
    let received = balance_delta(env, token_out_client, balance_before);

    // The aggregator payload owns slippage. The controller still requires an
    // observable positive output delta from the trusted aggregator call.
    assert_with_error!(env, received > 0, GenericError::InternalError);

    received
}

fn refund_controller_balance_delta(
    env: &Env,
    asset: &Address,
    balance_before: i128,
    refund_to: &Address,
) {
    let token = soroban_sdk::token::Client::new(env, asset);
    let excess = balance_delta(env, &token, balance_before);
    if excess > 0 {
        token.transfer(&env.current_contract_address(), refund_to, &excess);
    }
}

/// Token balance the controller gained since `balance_before`; a negative delta
/// is impossible from a sane token and panics.
fn balance_delta(env: &Env, token: &soroban_sdk::token::Client, balance_before: i128) -> i128 {
    token
        .balance(&env.current_contract_address())
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}

fn addresses_from_slice(env: &Env, items: &[&Address]) -> Vec<Address> {
    let mut out: Vec<Address> = Vec::new(env);
    for addr in items {
        utils::push_unique_address(&mut out, (*addr).clone());
    }
    out
}

// Touched-asset descriptor for the post-finalize dust gate: strategies list
// only the legs they mutated, so pre-existing positions that drifted under
// floor (price moves, accrual) don't block the call.
pub(crate) struct StrategyTouched<'a> {
    pub supply_assets: &'a [&'a Address],
    pub borrow_assets: &'a [&'a Address],
}

pub(crate) fn strategy_finalize(
    env: &Env,
    account_id: u64,
    account: &mut Account,
    cache: &mut Cache,
    touched: StrategyTouched<'_>,
) {
    // Remove accounts that closed out entirely; otherwise persist.
    if account.is_empty() {
        utils::remove_account(env, account_id);
    } else {
        // Strategy flows touch only the position maps, so flush each side
        // directly (avoids re-reading meta for an equality compare); each side
        // write TTL-bumps meta via `write_side_map`.
        storage::set_supply_positions(env, account_id, &account.supply_positions);
        storage::set_debt_positions(env, account_id, &account.borrow_positions);
    }

    // Re-check the LTV ceiling and health factor against the post-mutation
    // positions so a strategy cannot exit above the LTV ceiling. Prices are not
    // re-read: every oracle source is an external feed (Reflector/RedStone)
    // fixed for the ledger, so the strategy's own swap cannot move a valuation
    // price within this atomic tx — the cached prices equal a fresh read.
    validation::require_within_ltv(env, cache, account);
    validation::require_healthy_account(env, cache, account);
    // Enforce the per-asset dust floor on the legs this strategy mutated
    // (the entrypoint-built `touched` set), which may hold sub-floor residue.
    let supply_touched = addresses_from_slice(env, touched.supply_assets);
    let borrow_touched = addresses_from_slice(env, touched.borrow_assets);
    require_no_supply_dust_for_assets(env, cache, account, &supply_touched);
    require_no_borrow_dust_for_assets(env, cache, account, &borrow_touched);

    // Borrow-cap enforcement lives at the entrypoints that open debt (multiply,
    // swap_debt), mirroring `process_borrow`; debt-neutral strategies
    // (swap_collateral, repay_debt_with_collateral) skip it.

    cache.flush_isolated_debts();
    cache.emit_position_batch(account_id, account);
    cache.emit_market_batch();
}

pub(crate) fn execute_withdraw_all(
    env: &Env,
    account: &mut Account,
    destination: &Address,
    cache: &mut Cache,
) {
    // Collect keys to avoid borrowing issues during mutation.
    let deposit_keys: Vec<Address> = account.supply_positions.keys();
    for asset in deposit_keys.iter() {
        if let Some(pos) = account.supply_positions.get(asset.clone()) {
            let pos: AccountPosition = (&pos).into();
            let feed = cache.cached_price(&asset);
            // Same full-close signal as public `withdraw(..., amount: 0)`.
            withdraw::execute_withdrawal(
                env,
                account,
                EventContext {
                    caller: destination.clone(),
                    action: symbol_short!("close_wd"),
                },
                WithdrawalRequest {
                    asset: &asset,
                    amount: WITHDRAW_ALL_SENTINEL,
                    position: &pos,
                    price: feed.price,
                },
                WithdrawFlags::plain(),
                cache,
            );
        }
    }
}
