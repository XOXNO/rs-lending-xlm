use common::errors::GenericError;
use common::types::{Account, AccountPosition, AccountPositionType, AggregatorSwap, BatchSwap};
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{panic_with_error, symbol_short, Address, Env, IntoVal, Symbol, Vec};

use crate::cache::ControllerCache;
use crate::positions::dust::require_no_dust_after;
use crate::positions::repay::{self, RepaymentRequest};
use crate::positions::withdraw::{self, WithdrawFlags, WithdrawalRequest};
use crate::{positions::borrow, positions::EventContext, storage, utils, validation};

// Router client.
pub(crate) mod aggregator {
    use common::types::BatchSwap;
    use soroban_sdk::contractclient;

    #[allow(dead_code)]
    #[contractclient(name = "AggregatorClient")]
    pub trait Aggregator {
        fn batch_execute(env: soroban_sdk::Env, batch: BatchSwap) -> i128;
    }
}

pub(crate) struct StrategyRepay<'a> {
    pub debt_token: &'a Address,
    pub debt_available: i128,
    pub debt_pos: &'a AccountPosition,
    pub action: Symbol,
}

pub(crate) struct StrategyWithdraw<'a> {
    pub asset: &'a Address,
    pub amount: i128,
    pub position: &'a AccountPosition,
    pub action: Symbol,
}

pub(crate) struct SwapBalanceSnapshot {
    pub token_in: i128,
    pub token_out: i128,
}

pub(crate) fn controller_event_context(
    env: &Env,
    caller: &Address,
    action: Symbol,
) -> EventContext {
    EventContext {
        caller: env.current_contract_address(),
        event_caller: caller.clone(),
        action,
    }
}

pub(crate) fn open_strategy_borrow(
    env: &Env,
    cache: &mut ControllerCache,
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

    borrow::handle_create_borrow_strategy(env, cache, account, asset, amount)
}

pub(crate) fn repay_debt_from_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut ControllerCache,
    caller: &Address,
    req: StrategyRepay<'_>,
) {
    let debt_pool_addr = cache.cached_pool_address(req.debt_token);
    let debt_feed = cache.cached_price(req.debt_token);
    let debt_tok = soroban_sdk::token::Client::new(env, req.debt_token);

    // Pool-balance delta accounting around the transfer mirrors plain
    // `process_single_repay`: pass the amount that actually arrived to
    // `pool::repay`, not the requested `debt_available`. Defends against any
    // future onboarding of a fee-on-transfer or rebasing debt token where
    // `debt_available - fee` reaches the pool.
    let actual_arrived_at_pool = utils::transfer_and_measure_received(
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
        controller_event_context(env, caller, req.action),
        RepaymentRequest {
            asset: req.debt_token,
            position: req.debt_pos,
            amount: actual_arrived_at_pool,
            price: debt_feed.price,
        },
        cache,
    );

    refund_controller_balance_delta(env, req.debt_token, controller_balance_before_repay, caller);
}

pub(crate) fn withdraw_collateral_to_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut ControllerCache,
    caller: &Address,
    req: StrategyWithdraw<'_>,
) -> i128 {
    let feed = cache.cached_price(req.asset);
    let token = soroban_sdk::token::Client::new(env, req.asset);
    let balance_before = token.balance(&env.current_contract_address());

    withdraw::execute_withdrawal(
        env,
        account,
        controller_event_context(env, caller, req.action),
        WithdrawalRequest {
            asset: req.asset,
            amount: req.amount,
            position: req.position,
            price: feed.price,
        },
        WithdrawFlags::plain(),
        cache,
    );

    token
        .balance(&env.current_contract_address())
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}

// `_refund_to` is unused: router-leftover input stays on the controller
// and is refunded by `refund_controller_balance_delta` at the strategy tail.
pub(crate) fn swap_tokens(
    env: &Env,
    token_in: &Address,
    amount_in: i128,
    token_out: &Address,
    swap: &AggregatorSwap,
    _refund_to: &Address,
) -> i128 {
    let router_addr = storage::get_aggregator(env);
    let router = aggregator::AggregatorClient::new(env, &router_addr);
    let token_out_client = soroban_sdk::token::Client::new(env, token_out);
    let token_in_client = soroban_sdk::token::Client::new(env, token_in);

    // Validates swap commitment.
    validate_aggregator_swap(env, swap, token_in, token_out, amount_in);

    // Snapshot balances so `verify_router_output` can confirm the exact
    // delta the router pushed back. The router is trusted, but a defensive
    // delta check catches future ABI drift.
    let balance_before = snapshot_swap_balances(env, &token_in_client, &token_out_client);

    // Build the on-the-wire batch. `sender` is forced to the controller —
    // user input never sets it, eliminating any spoof path. `total_in` is
    // the controller's authoritative withdrawal amount; the router slices
    // it across paths via each path's `split_ppm`.
    // referral_id = 0 disables fees on the router.
    let batch = BatchSwap {
        paths: swap.paths.clone(),
        referral_id: 0,
        sender: env.current_contract_address(),
        total_in: amount_in,
        total_min_out: swap.total_min_out,
    };

    // Pre-authorize the router's input-token pull from the controller. Other
    // router transfers are initiated by the router itself and are covered by
    // direct-caller attestation.
    pre_authorize_router_pulls(env, &router_addr, &batch);

    call_router_with_reentrancy_guard(env, &router, &batch);

    // Defense-in-depth: reject any router pull above the controller's
    // committed input amount. Underspend remains on the controller and the
    // output-side minimum guards the received amount.
    verify_router_input_spend(env, &token_in_client, balance_before.token_in, amount_in);

    // The router enforces `total_out >= total_min_out` internally and
    // would have panicked otherwise. Re-check the controller-side
    // balance delta both as a sanity assertion and as the strategy's
    // primary slippage guard — strategy entrypoints already reject
    // `total_min_out <= 0` upfront.
    verify_router_output(
        env,
        &token_out_client,
        balance_before.token_out,
        swap.total_min_out,
    )
}

// Rejects batches that don't match strategy commitment.
pub(crate) fn validate_aggregator_swap(
    env: &Env,
    swap: &AggregatorSwap,
    token_in: &Address,
    token_out: &Address,
    amount_in: i128,
) {
    // Empty batch, empty path, and wrong-token batches are caller mistakes.
    // Report them as `InvalidPayments`, mirroring the rest of the
    // controller's "malformed input" surface.
    if swap.paths.is_empty() {
        panic_with_error!(env, GenericError::InvalidPayments);
    }
    if amount_in <= 0 || swap.total_min_out <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }

    // Per-path validation: each path must declare a non-zero PPM share,
    // start at `token_in` and end at `token_out`. The router computes the
    // per-path input as `amount_in * split_ppm / 1_000_000` (last path
    // absorbs PPM rounding) so there are no per-path amount fields here
    // to validate. Any rounding drift between off-chain quote and
    // on-chain settlement is irrelevant: the controller sources `amount_in`
    // from its actual withdrawal delta, never from the quote.
    let mut sum_ppm: u32 = 0;
    let n = swap.paths.len();
    for i in 0..n {
        let path = validation::expect_invariant(env, swap.paths.get(i));
        if path.hops.is_empty() {
            panic_with_error!(env, GenericError::InvalidPayments);
        }
        if path.split_ppm == 0 {
            panic_with_error!(env, GenericError::InvalidPayments);
        }
        sum_ppm = sum_ppm
            .checked_add(path.split_ppm)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::InvalidPayments));

        let first_hop = validation::expect_invariant(env, path.hops.get(0));
        if first_hop.token_in != *token_in {
            panic_with_error!(env, GenericError::WrongToken);
        }
        let last_hop = validation::expect_invariant(env, path.hops.get(path.hops.len() - 1));
        if last_hop.token_out != *token_out {
            panic_with_error!(env, GenericError::WrongToken);
        }
    }
    // PPM weights must sum to exactly 1_000_000. Anything else means the
    // off-chain quote was malformed; the router would also reject this
    // but failing fast in the controller keeps the panic site close to
    // the user-visible call.
    if sum_ppm != 1_000_000 {
        panic_with_error!(env, GenericError::InvalidPayments);
    }
}

pub(crate) fn snapshot_swap_balances(
    env: &Env,
    token_in_client: &soroban_sdk::token::Client,
    token_out_client: &soroban_sdk::token::Client,
) -> SwapBalanceSnapshot {
    SwapBalanceSnapshot {
        token_in: token_in_client.balance(&env.current_contract_address()),
        token_out: token_out_client.balance(&env.current_contract_address()),
    }
}

// Invokes the router under the flash-loan guard. Save+restore preserves
// any pre-existing outer guard.
pub(crate) fn call_router_with_reentrancy_guard(
    env: &Env,
    router: &aggregator::AggregatorClient,
    batch: &BatchSwap,
) {
    let previously_set = storage::is_flash_loan_ongoing(env);
    storage::set_flash_loan_ongoing(env, true);
    let _ = router.batch_execute(batch);
    if !previously_set {
        storage::set_flash_loan_ongoing(env, false);
    }
}

// Authorizes router token pull.
pub(crate) fn pre_authorize_router_pulls(env: &Env, router_addr: &Address, batch: &BatchSwap) {
    let first_path = validation::expect_invariant(env, batch.paths.get(0));
    let first_hop = validation::expect_invariant(env, first_path.hops.get(0));
    let entry = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: first_hop.token_in.clone(),
            fn_name: symbol_short!("transfer"),
            args: (
                env.current_contract_address(),
                router_addr.clone(),
                batch.total_in,
            )
                .into_val(env),
        },
        sub_invocations: Vec::new(env),
    });
    let mut entries: Vec<InvokerContractAuthEntry> = Vec::new(env);
    entries.push_back(entry);
    env.authorize_as_current_contract(entries);
}

// Rejects overspend by router.
pub(crate) fn verify_router_input_spend(
    env: &Env,
    token_in_client: &soroban_sdk::token::Client,
    balance_before: i128,
    amount_in: i128,
) {
    let balance_after = token_in_client.balance(&env.current_contract_address());
    if balance_after > balance_before {
        panic_with_error!(env, GenericError::InternalError);
    }
    let actual_in_spent = balance_before - balance_after;
    // Allow the router to spend less than `amount_in`; leftover input stays on
    // the controller and output verification still enforces `total_min_out`.
    // Reject overspend because it signals that the router or token contract
    // pulled more than the strategy committed to spend.
    if actual_in_spent > amount_in {
        panic_with_error!(env, GenericError::InternalError);
    }
}

pub(crate) fn verify_router_output(
    env: &Env,
    token_out_client: &soroban_sdk::token::Client,
    balance_before: i128,
    total_min_out: i128,
) -> i128 {
    // Received must be non-negative. A decrease is impossible from a sane
    // token contract and indicates aggregator or token misbehavior.
    let received = token_out_client
        .balance(&env.current_contract_address())
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));

    // Defense-in-depth slippage check at the controller. The router
    // already enforces `total_out >= total_min_out` and would have
    // panicked otherwise. Strategy entrypoints reject
    // `total_min_out <= 0` upfront.
    if received < total_min_out {
        panic_with_error!(env, GenericError::InternalError);
    }

    received
}

pub(crate) fn refund_controller_balance_delta(
    env: &Env,
    asset: &Address,
    balance_before: i128,
    refund_to: &Address,
) {
    let token = soroban_sdk::token::Client::new(env, asset);
    let balance_after = token.balance(&env.current_contract_address());
    let excess = balance_after
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    if excess > 0 {
        token.transfer(&env.current_contract_address(), refund_to, &excess);
    }
}

// Persists state and flushes isolated debt.
pub(crate) fn strategy_finalize(
    env: &Env,
    account_id: u64,
    account: &mut Account,
    cache: &mut ControllerCache,
) {
    // Remove accounts that closed out entirely; otherwise persist.
    if account.is_empty() {
        utils::remove_account(env, account_id);
    } else {
        // Strategy flows mutate only the position maps; meta fields stay
        // exactly as loaded. Flush sides directly so the meta key is not
        // re-read for an equality compare. Each side write TTL-bumps meta
        // via `write_side_map`.
        storage::set_positions(
            env,
            account_id,
            AccountPositionType::Deposit,
            &account.supply_positions,
        );
        storage::set_positions(
            env,
            account_id,
            AccountPositionType::Borrow,
            &account.borrow_positions,
        );
    }

    // Re-check HF (against liquidation threshold) and LTV (against
    // borrow capacity) with a fresh price cache after the leveraged
    // mutation. LTV must be re-checked on every collateral-reducing
    // or debt-shifting path so a strategy cannot exit the call above
    // the configured LTV ceiling.
    cache.prices_cache = soroban_sdk::Map::new(env);
    validation::require_within_ltv(env, cache, account);
    validation::require_healthy_account(env, cache, account);
    // Strategy paths can leave sub-floor residue on either side; the
    // per-asset dust floor is enforced on every position post-finalize.
    require_no_dust_after(env, cache, account);

    // Borrow-position-cap enforcement lives at each strategy entrypoint
    // that actually opens debt (multiply, swap_debt) — mirrors `borrow_batch`'s
    // upfront check. Strategies that don't open debt (swap_collateral,
    // repay_debt_with_collateral) skip the cap check.

    cache.flush_isolated_debts();
    cache.emit_position_batch(account_id, account);
    cache.emit_market_batch();
}

// Withdraws all supply positions to destination.
pub(crate) fn execute_withdraw_all(
    env: &Env,
    account: &mut Account,
    account_id: u64,
    destination: &Address,
    cache: &mut ControllerCache,
) {
    let _ = account_id;
    // Collect keys to avoid borrowing issues during mutation.
    let deposit_keys: Vec<Address> = account.supply_positions.keys();
    for i in 0..deposit_keys.len() {
        let asset = validation::expect_invariant(env, deposit_keys.get(i));
        if let Some(pos) = account.supply_positions.get(asset.clone()) {
            let pos: AccountPosition = (&pos).into();
            let feed = cache.cached_price(&asset);
            let market_index = cache.cached_market_index(&asset);
            let full_amount = pos
                .scaled_amount
                .mul(env, market_index.supply_index)
                .to_asset(feed.asset_decimals);
            // `destination` doubles as `event_caller` — the user initiated this close.
            let _updated = withdraw::execute_withdrawal(
                env,
                account,
                EventContext {
                    caller: destination.clone(),
                    event_caller: destination.clone(),
                    action: symbol_short!("close_wd"),
                },
                WithdrawalRequest {
                    asset: &asset,
                    amount: full_amount,
                    position: &pos,
                    price: feed.price,
                },
                WithdrawFlags::plain(),
                cache,
            );
        }
    }
}
