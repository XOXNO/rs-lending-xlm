use common::errors::{CollateralError, EModeError, FlashLoanError, GenericError, StrategyError};
use common::events::{emit_initial_multiply_payment, InitialMultiplyPaymentEvent};
use common::fp::{Ray, Wad};
use common::types::{
    Account, AccountPosition, AssetConfig, PositionMode, SwapSteps, POSITION_TYPE_BORROW,
    POSITION_TYPE_DEPOSIT,
};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Symbol, Vec};
use stellar_macros::when_not_paused;

use crate::cache::ControllerCache;
use crate::{
    positions::{borrow, emode, repay, supply, withdraw, EventContext},
    storage, utils, validation, Controller, ControllerArgs, ControllerClient,
};

mod aggregator {
    use common::types::DexDistribution;
    use soroban_sdk::{contractclient, Address, Vec};

    #[allow(dead_code)]
    #[contractclient(name = "AggregatorClient")]
    pub trait Aggregator {
        #[allow(clippy::too_many_arguments)]
        fn swap_exact_tokens_for_tokens(
            env: soroban_sdk::Env,
            token_in: Address,
            token_out: Address,
            amount_in: i128,
            amount_out_min: i128,
            distribution: Vec<DexDistribution>,
            to: Address,
            deadline: u64,
        ) -> Vec<Vec<i128>>;
    }
}

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn multiply(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        collateral_token: Address,
        debt_to_flash_loan: i128,
        debt_token: Address,
        mode: PositionMode,
        steps: SwapSteps,
        initial_payment: Option<(Address, i128)>,
        convert_steps: Option<SwapSteps>,
    ) -> u64 {
        process_multiply(
            &env,
            &caller,
            account_id,
            e_mode_category,
            &collateral_token,
            debt_to_flash_loan,
            &debt_token,
            mode,
            &steps,
            initial_payment,
            convert_steps,
        )
    }

    #[when_not_paused]
    pub fn swap_debt(
        env: Env,
        caller: Address,
        account_id: u64,
        existing_debt_token: Address,
        amount: i128,
        new_debt_token: Address,
        steps: SwapSteps,
    ) {
        process_swap_debt(
            &env,
            &caller,
            account_id,
            &existing_debt_token,
            amount,
            &new_debt_token,
            &steps,
        );
    }

    #[when_not_paused]
    pub fn swap_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        current_collateral: Address,
        amount: i128,
        new_collateral: Address,
        steps: SwapSteps,
    ) {
        process_swap_collateral(
            &env,
            &caller,
            account_id,
            &current_collateral,
            amount,
            &new_collateral,
            &steps,
        );
    }

    #[when_not_paused]
    pub fn repay_debt_with_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        collateral_token: Address,
        collateral_amount: i128,
        debt_token: Address,
        steps: SwapSteps,
        close_position: bool,
    ) {
        process_repay_debt_with_collateral(
            &env,
            &caller,
            account_id,
            &collateral_token,
            collateral_amount,
            &debt_token,
            &steps,
            close_position,
        );
    }
}

// ---------------------------------------------------------------------------
// Multiply (Leverage)
// ---------------------------------------------------------------------------

/// Opens a leveraged position: borrows `debt_to_flash_loan` via the pool flash strategy,
/// swaps to `collateral_token`, and deposits the proceeds. Accepts an optional initial payment
/// to increase collateral or reduce the required flash-loan amount.
pub fn process_multiply(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    collateral_token: &Address,
    debt_to_flash_loan: i128,
    debt_token: &Address,
    mode: PositionMode,
    steps: &SwapSteps,
    initial_payment: Option<(Address, i128)>,
    convert_steps: Option<SwapSteps>,
) -> u64 {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    if collateral_token == debt_token {
        panic_with_error!(env, GenericError::AssetsAreTheSame);
    }

    // Allow-list rather than `!= Normal` so a future `PositionMode` variant
    // cannot silently slip through multiply.
    if !matches!(
        mode,
        PositionMode::Multiply | PositionMode::Long | PositionMode::Short
    ) {
        panic_with_error!(env, CollateralError::InvalidPositionMode);
    }

    validation::require_amount_positive(env, debt_to_flash_loan);
    // Reject zero-floor swap requests at entry so a compromised router
    // cannot observe an unprotected slippage floor.
    validation::require_amount_positive(env, steps.amount_out_min);

    let (collateral_amount, debt_extra) = collect_initial_multiply_payment(
        env,
        caller,
        collateral_token,
        debt_token,
        &initial_payment,
        &convert_steps,
    );

    // Strict-price cache: strategy borrows are risk-increasing.
    let mut cache = ControllerCache::new(env, false);

    let collateral_config = cache.cached_asset_config(collateral_token);
    if !collateral_config.is_collateralizable {
        panic_with_error!(env, CollateralError::NotCollateral);
    }

    let (account_id, mut account) = load_or_create_multiply_account(
        env,
        caller,
        account_id,
        e_mode_category,
        collateral_token,
        &collateral_config,
        mode,
    );

    let amount_received = open_strategy_borrow(
        env,
        &mut cache,
        &mut account,
        account_id,
        debt_token,
        debt_to_flash_loan,
        caller,
    );

    let swap_amount_in = amount_received
        .checked_add(debt_extra)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let swapped_collateral = swap_tokens(
        env,
        debt_token,
        swap_amount_in,
        collateral_token,
        steps,
        caller,
    );

    let total_collateral = collateral_amount
        .checked_add(swapped_collateral)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    let mut deposit_assets = Vec::new(env);
    deposit_assets.push_back((collateral_token.clone(), total_collateral));
    supply::process_deposit(
        env,
        &env.current_contract_address(),
        account_id,
        &mut account,
        &deposit_assets,
        &mut cache,
    );

    strategy_finalize(env, account_id, &mut account, &mut cache);

    emit_multiply_initial_payment(env, &mut cache, account_id, initial_payment);

    account_id
}

// ---------------------------------------------------------------------------
// Swap Debt
// ---------------------------------------------------------------------------

/// Swaps an existing debt position to a new token: borrows the new token via the pool flash
/// strategy, swaps through the aggregator, and repays the old debt.
pub fn process_swap_debt(
    env: &Env,
    caller: &Address,
    account_id: u64,
    existing_debt_token: &Address,
    new_debt_amount: i128,
    new_debt_token: &Address,
    steps: &SwapSteps,
) {
    validation::require_not_flash_loaning(env);

    if existing_debt_token == new_debt_token {
        panic_with_error!(env, GenericError::AssetsAreTheSame);
    }

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);

    // Strict-price cache: strategy borrows are risk-increasing.
    let mut cache = ControllerCache::new(env, false);

    validation::require_amount_positive(env, new_debt_amount);
    // Reject zero-floor swap requests at entry.
    validation::require_amount_positive(env, steps.amount_out_min);

    // Reject swap_debt when either side is siloed: the flow holds both debt
    // positions simultaneously (new is borrowed before old is repaid),
    // which violates the siloed-borrow invariant.
    let existing_debt_config = cache.cached_asset_config(existing_debt_token);
    let new_debt_config = cache.cached_asset_config(new_debt_token);
    if existing_debt_config.is_siloed_borrowing || new_debt_config.is_siloed_borrowing {
        panic_with_error!(env, CollateralError::NotBorrowableSiloed);
    }

    let amount_received = open_strategy_borrow(
        env,
        &mut cache,
        &mut account,
        account_id,
        new_debt_token,
        new_debt_amount,
        caller,
    );

    let swapped_amount = swap_tokens(
        env,
        new_debt_token,
        amount_received,
        existing_debt_token,
        steps,
        caller,
    );

    let existing_pos = account
        .borrow_positions
        .get(existing_debt_token.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound));

    repay_debt_from_controller(
        env,
        &mut account,
        &mut cache,
        caller,
        existing_debt_token,
        swapped_amount,
        &existing_pos,
        symbol_short!("sw_debt_r"),
    );

    strategy_finalize(env, account_id, &mut account, &mut cache);
}

// ---------------------------------------------------------------------------
// Swap Collateral
// ---------------------------------------------------------------------------

/// Swaps existing collateral to a different token: withdraws `from_amount`, swaps through
/// the aggregator, and re-deposits the proceeds as the new collateral.
pub fn process_swap_collateral(
    env: &Env,
    caller: &Address,
    account_id: u64,
    current_collateral: &Address,
    from_amount: i128,
    new_collateral: &Address,
    steps: &SwapSteps,
) {
    validation::require_not_flash_loaning(env);

    if current_collateral == new_collateral {
        panic_with_error!(env, GenericError::AssetsAreTheSame);
    }

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);

    if account.is_isolated {
        panic_with_error!(env, FlashLoanError::SwapCollateralNoIso);
    }

    // Debt-free collateral swaps are risk-neutral; the tightest oracle
    // tolerance is unnecessary when no outstanding borrows can be liquidated.
    let allow_unsafe_price = account.borrow_positions.is_empty();
    let mut cache = ControllerCache::new(env, allow_unsafe_price);

    validation::require_amount_positive(env, from_amount);
    // Reject zero-floor swap requests at entry.
    validation::require_amount_positive(env, steps.amount_out_min);

    validate_swap_new_collateral_preflight(env, &mut cache, &account, new_collateral);

    let current_pos = account
        .supply_positions
        .get(current_collateral.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::CollateralPositionNotFound));

    let actual_withdrawn = withdraw_collateral_to_controller(
        env,
        &mut account,
        &mut cache,
        caller,
        current_collateral,
        from_amount,
        &current_pos,
        symbol_short!("sw_col_wd"),
    );

    let swapped_amount = swap_tokens(
        env,
        current_collateral,
        actual_withdrawn,
        new_collateral,
        steps,
        caller,
    );

    let mut deposit_assets = Vec::new(env);
    deposit_assets.push_back((new_collateral.clone(), swapped_amount));
    supply::process_deposit(
        env,
        &env.current_contract_address(),
        account_id,
        &mut account,
        &deposit_assets,
        &mut cache,
    );

    strategy_finalize(env, account_id, &mut account, &mut cache);
}

// ---------------------------------------------------------------------------
// Swap router helper
// ---------------------------------------------------------------------------

fn swap_tokens(
    env: &Env,
    token_in: &Address,
    amount_in: i128,
    token_out: &Address,
    steps: &SwapSteps,
    refund_to: &Address,
) -> i128 {
    let router_addr = storage::get_aggregator(env);
    let router = aggregator::AggregatorClient::new(env, &router_addr);
    let token_out_client = soroban_sdk::token::Client::new(env, token_out);
    let token_in_client = soroban_sdk::token::Client::new(env, token_in);

    // Snapshot controller balances on both sides before any approvals to
    // verify exact spend and received amounts against a misbehaving router.
    let balance_before = snapshot_swap_balances(env, &token_in_client, &token_out_client);

    approve_router_input(env, &token_in_client, &router_addr, amount_in);
    call_router_with_reentrancy_guard(env, &router, token_in, token_out, amount_in, steps);

    settle_router_input(
        env,
        &token_in_client,
        &router_addr,
        amount_in,
        balance_before.token_in,
        refund_to,
    );
    

    // Note: any third-party token the router happens to deposit into the
    // controller (LP rebate, governance reward, malformed output) is NOT
    // swept here. With SEP-41 push-on-transfer semantics only the
    // configured router can land tokens, and only `token_in` and
    // `token_out` are part of this swap's contract. Adding a generic sweep
    // would require an oracle of "expected output tokens" which the
    // strategy callsites already supply via `token_out`. If a future
    // aggregator integration emits multi-token output, expand the
    // signature with a `&Vec<Address>` of expected-zero-delta tokens.
    verify_router_output(
        env,
        &token_out_client,
        balance_before.token_out,
        steps.amount_out_min,
    )
}

// ---------------------------------------------------------------------------
// Repay Debt With Collateral
// ---------------------------------------------------------------------------

/// Withdraws collateral, swaps it to the debt token via the aggregator, and repays debt.
/// When `close_position` is true, withdraws all remaining collateral to the caller after repayment.
pub fn process_repay_debt_with_collateral(
    env: &Env,
    caller: &Address,
    account_id: u64,
    collateral_token: &Address,
    collateral_amount: i128,
    debt_token: &Address,
    steps: &SwapSteps,
    close_position: bool,
) {
    validation::require_not_flash_loaning(env);
    validation::require_amount_positive(env, collateral_amount);
    // Skip the slippage-floor check for the same-asset short-circuit (no
    // swap occurs).
    if collateral_token != debt_token {
        validation::require_amount_positive(env, steps.amount_out_min);
    }

    // The same-asset flow is intentionally allowed: self-collateralized
    // positions (e.g. stablecoin/stablecoin) can net the two legs atomically
    // without routing through the aggregator.

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);

    let mut cache = ControllerCache::new(env, false);

    let (collateral_pos, debt_pos) =
        load_repay_with_collateral_positions(env, &account, collateral_token, debt_token);

    let actual_withdrawn = withdraw_collateral_to_controller(
        env,
        &mut account,
        &mut cache,
        caller,
        collateral_token,
        collateral_amount,
        &collateral_pos,
        symbol_short!("rp_col_wd"),
    );

    let debt_available = swap_or_net_collateral_to_debt(
        env,
        caller,
        collateral_token,
        debt_token,
        actual_withdrawn,
        steps,
    );
    repay_debt_from_controller(
        env,
        &mut account,
        &mut cache,
        caller,
        debt_token,
        debt_available,
        &debt_pos,
        symbol_short!("rp_col_r"),
    );

    close_remaining_collateral_if_requested(env, &mut account, caller, &mut cache, close_position);

    strategy_finalize(env, account_id, &mut account, &mut cache);
}

// ---------------------------------------------------------------------------
// Strategy Helpers
// ---------------------------------------------------------------------------

fn controller_event_context(env: &Env, caller: &Address, action: Symbol) -> EventContext {
    EventContext {
        caller: env.current_contract_address(),
        event_caller: caller.clone(),
        action,
    }
}

struct SwapBalanceSnapshot {
    token_in: i128,
    token_out: i128,
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

fn approve_router_input(
    env: &Env,
    token_in_client: &soroban_sdk::token::Client,
    router_addr: &Address,
    amount_in: i128,
) {
    token_in_client.approve(
        &env.current_contract_address(),
        router_addr,
        &amount_in,
        &env.ledger()
            .sequence()
            .checked_add(200)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
    );
}

fn call_router_with_reentrancy_guard(
    env: &Env,
    router: &aggregator::AggregatorClient,
    token_in: &Address,
    token_out: &Address,
    amount_in: i128,
    steps: &SwapSteps,
) {
    // Reuse the flash-loan-ongoing flag as a re-entry guard: a misbehaving
    // aggregator callback into any mutating controller endpoint trips the
    // mutator's `require_not_flash_loaning` and panics. The flag is FALSE
    // on entry because strategies never run inside a flash loan.
    storage::set_flash_loan_ongoing(env, true);

    let _ = router.swap_exact_tokens_for_tokens(
        token_in,
        token_out,
        &amount_in,
        &steps.amount_out_min,
        &steps.distribution,
        &env.current_contract_address(),
        &env.ledger()
            .timestamp()
            .checked_add(3600)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
    );

    storage::set_flash_loan_ongoing(env, false);
}

fn settle_router_input(
    env: &Env,
    token_in_client: &soroban_sdk::token::Client,
    router_addr: &Address,
    amount_in: i128,
    balance_before: i128,
    refund_to: &Address,
) {
    // Verify the exact input spend. A well-behaved router pulls at most
    // `amount_in` and never returns tokens (balance going UP). Either
    // direction of misbehavior is an internal error.
    let actual_in_spent =
        verify_router_input_spend(env, token_in_client, balance_before, amount_in);

    // Zero any residual allowance so a compromised or lazy router cannot
    // pull additional funds after the swap returns.
    token_in_client.approve(&env.current_contract_address(), router_addr, &0, &0);

    // Refund any unspent `token_in` to `refund_to` (= original user). A
    // router that partial-fills (route exhaustion, integer rounding, or a
    // bug) leaves residual `token_in` on the controller; the SEP-41
    // approve+pull model provides no callback-returned transfer bundle, so
    // unspent input requires manual reconciliation.
    let unspent = amount_in - actual_in_spent;
    if unspent > 0 {
        token_in_client.transfer(&env.current_contract_address(), refund_to, &unspent);
    }
}

fn verify_router_input_spend(
    env: &Env,
    token_in_client: &soroban_sdk::token::Client,
    balance_before: i128,
    amount_in: i128,
) -> i128 {
    let balance_after = token_in_client.balance(&env.current_contract_address());
    if balance_after > balance_before {
        panic_with_error!(env, GenericError::InternalError);
    }
    let actual_in_spent = balance_before - balance_after;
    if actual_in_spent > amount_in {
        panic_with_error!(env, GenericError::InternalError);
    }
    actual_in_spent
}

fn verify_router_output(
    env: &Env,
    token_out_client: &soroban_sdk::token::Client,
    balance_before: i128,
    amount_out_min: i128,
) -> i128 {
    // Received must be non-negative. A decrease is impossible from a sane
    // token contract and indicates aggregator or token misbehavior.
    let received = token_out_client
        .balance(&env.current_contract_address())
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));

    // Enforce the slippage minimum at the controller so a router that
    // ignores its own `amount_out_min` cannot silently shortchange the
    // caller. Strategy entrypoints already reject `amount_out_min <= 0`.
    if received < amount_out_min {
        panic_with_error!(env, GenericError::InternalError);
    }

    received
}

fn collect_initial_multiply_payment(
    env: &Env,
    caller: &Address,
    collateral_token: &Address,
    debt_token: &Address,
    initial_payment: &Option<(Address, i128)>,
    convert_steps: &Option<SwapSteps>,
) -> (i128, i128) {
    let mut collateral_amount = 0;
    let mut debt_extra = 0;

    if let Some((payment_token, payment_amount)) = initial_payment.as_ref() {
        validation::require_amount_positive(env, *payment_amount);

        let payment_tok = soroban_sdk::token::Client::new(env, payment_token);
        payment_tok.transfer(caller, env.current_contract_address(), payment_amount);

        if *payment_token == *collateral_token {
            collateral_amount = *payment_amount;
        } else if *payment_token == *debt_token {
            debt_extra = *payment_amount;
        } else {
            let convert = match convert_steps.as_ref() {
                Some(steps) => steps,
                None => panic_with_error!(env, StrategyError::ConvertStepsRequired),
            };
            collateral_amount = swap_tokens(
                env,
                payment_token,
                *payment_amount,
                collateral_token,
                convert,
                caller,
            );
        }
    }

    (collateral_amount, debt_extra)
}

fn load_or_create_multiply_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    collateral_token: &Address,
    collateral_config: &AssetConfig,
    mode: PositionMode,
) -> (u64, Account) {
    if account_id == 0 {
        let is_isolated = collateral_config.is_isolated_asset;
        let isolated_asset = if is_isolated {
            Some(collateral_token.clone())
        } else {
            None
        };
        // `create_account` returns the in-memory snapshot it just wrote, so
        // there's no need to re-read all 3 keys from storage.
        return utils::create_account(
            env,
            caller,
            e_mode_category,
            mode,
            is_isolated,
            isolated_asset,
        );
    }

    let account = storage::get_account(env, account_id);
    validation::require_account_owner_match(env, &account, caller);
    if account.mode != mode {
        panic_with_error!(env, GenericError::AccountModeMismatch);
    }
    (account_id, account)
}

fn open_strategy_borrow(
    env: &Env,
    cache: &mut ControllerCache,
    account: &mut Account,
    account_id: u64,
    asset: &Address,
    amount: i128,
    caller: &Address,
) -> i128 {
    let new_borrow_assets = soroban_sdk::vec![env, (asset.clone(), amount)];
    validation::validate_bulk_position_limits(
        env,
        account,
        POSITION_TYPE_BORROW,
        &new_borrow_assets,
    );

    borrow::handle_create_borrow_strategy(env, cache, account, account_id, asset, amount, caller)
}

fn emit_multiply_initial_payment(
    env: &Env,
    cache: &mut ControllerCache,
    account_id: u64,
    initial_payment: Option<(Address, i128)>,
) {
    if let Some((payment_token, payment_amount)) = initial_payment {
        let feed = cache.cached_price(&payment_token);
        let amount_wad = Wad::from_token(payment_amount, feed.asset_decimals);
        let usd_value_wad = amount_wad.mul(env, Wad::from_raw(feed.price_wad)).raw();
        emit_initial_multiply_payment(
            env,
            InitialMultiplyPaymentEvent {
                token: payment_token,
                amount: payment_amount,
                usd_value_wad,
                account_id,
            },
        );
    }
}

fn refund_controller_balance_delta(
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

fn load_repay_with_collateral_positions(
    env: &Env,
    account: &Account,
    collateral_token: &Address,
    debt_token: &Address,
) -> (AccountPosition, AccountPosition) {
    // Validate both positions before moving any tokens so a missing
    // position surfaces as its specific error rather than a host panic on
    // a later transfer.
    let collateral_pos = account
        .supply_positions
        .get(collateral_token.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::CollateralPositionNotFound));
    let debt_pos = account
        .borrow_positions
        .get(debt_token.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound));

    (collateral_pos, debt_pos)
}

fn swap_or_net_collateral_to_debt(
    env: &Env,
    caller: &Address,
    collateral_token: &Address,
    debt_token: &Address,
    collateral_amount: i128,
    steps: &SwapSteps,
) -> i128 {
    if collateral_token == debt_token {
        return collateral_amount;
    }

    swap_tokens(
        env,
        collateral_token,
        collateral_amount,
        debt_token,
        steps,
        caller,
    )
}

fn repay_debt_from_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut ControllerCache,
    caller: &Address,
    debt_token: &Address,
    debt_available: i128,
    debt_pos: &AccountPosition,
    action: Symbol,
) {
    let debt_pool_addr = cache.cached_pool_address(debt_token);
    let debt_feed = cache.cached_price(debt_token);
    let debt_tok = soroban_sdk::token::Client::new(env, debt_token);

    // Pool-balance delta accounting around the transfer mirrors plain
    // `process_single_repay`: pass the amount that actually arrived to
    // `pool::repay`, not the requested `debt_available`. Defends against any
    // future onboarding of a fee-on-transfer or rebasing debt token where
    // `debt_available - fee` reaches the pool.
    let actual_arrived_at_pool = utils::transfer_and_measure_received(
        env,
        debt_token,
        &env.current_contract_address(),
        &debt_pool_addr,
        debt_available,
        GenericError::InternalError,
    );

    let controller_balance_before_repay = debt_tok.balance(&env.current_contract_address());

    // Routes through the shared repay path for isolated-debt handling.
    repay::execute_repayment(
        env,
        account,
        controller_event_context(env, caller, action),
        debt_pos,
        debt_feed.price_wad,
        actual_arrived_at_pool,
        cache,
    );

    refund_controller_balance_delta(env, debt_token, controller_balance_before_repay, caller);
}

fn close_remaining_collateral_if_requested(
    env: &Env,
    account: &mut Account,
    caller: &Address,
    cache: &mut ControllerCache,
    close_position: bool,
) {
    if !close_position {
        return;
    }

    if !account.borrow_positions.is_empty() {
        panic_with_error!(env, CollateralError::CannotCloseWithRemainingDebt);
    }

    execute_withdraw_all(env, account, caller, cache);
}

fn withdraw_collateral_to_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut ControllerCache,
    caller: &Address,
    asset: &Address,
    amount: i128,
    position: &AccountPosition,
    action: Symbol,
) -> i128 {
    let feed = cache.cached_price(asset);
    let token = soroban_sdk::token::Client::new(env, asset);
    let balance_before = token.balance(&env.current_contract_address());

    withdraw::execute_withdrawal(
        env,
        account,
        controller_event_context(env, caller, action),
        amount,
        position,
        false,
        0,
        feed.price_wad,
        cache,
    );

    token
        .balance(&env.current_contract_address())
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}

/// Persists account state, re-checks HF with a fresh price cache, and flushes isolated-debt.
/// Deletes the account when all positions close on an owner-initiated full exit.
pub fn strategy_finalize(
    env: &Env,
    account_id: u64,
    account: &mut Account,
    cache: &mut ControllerCache,
) {
    // Remove accounts that closed out entirely; otherwise persist.
    //
    // Intentional asymmetry with plain `process_repay`: the plain repay path
    // never deletes an account on full debt close, even when both maps go
    // empty (anti-grief — a third-party repaying your last debt cannot make
    // your `account_id` disappear mid-block). Strategy paths are different:
    // `repay_debt_with_collateral` with `close_position=true` is an
    // owner-initiated full close where the same caller withdraws all
    // collateral within the same atomic call, so the account is genuinely
    // empty by the user's own request. `multiply` / `swap_debt` /
    // `swap_collateral` reach the empty-empty state only on revert paths,
    // which Soroban rolls back atomically. Deleting here avoids leaving empty
    // account storage after successful close flows.
    if account.supply_positions.is_empty() && account.borrow_positions.is_empty() {
        utils::remove_account(env, account_id);
    } else {
        // Strategy flows mutate only the position maps; meta fields stay
        // exactly as loaded. Flush sides directly so the meta key is not
        // re-read for an equality compare. Each side write TTL-bumps meta
        // via `write_side_map`.
        storage::set_supply_positions(env, account_id, &account.supply_positions);
        storage::set_borrow_positions(env, account_id, &account.borrow_positions);
    }

    // Re-check HF with a fresh price cache after the leveraged mutation.
    cache.clean_prices_cache();
    validation::require_healthy_account(env, cache, account);

    // Borrow-position-cap enforcement lives at each strategy entrypoint
    // that actually opens debt (multiply, swap_debt) — mirrors `borrow_batch`'s
    // upfront check. Strategies that don't open debt (swap_collateral,
    // repay_debt_with_collateral) skip the cap check.

    cache.flush_isolated_debts();
}

/// Withdraws the full balance of every supply position to `destination`.
/// Used by `process_repay_debt_with_collateral` for the close-position leg.
pub fn execute_withdraw_all(
    env: &Env,
    account: &mut Account,
    destination: &Address,
    cache: &mut ControllerCache,
) {
    // Collect keys to avoid borrowing issues during mutation.
    let deposit_keys: Vec<Address> = account.supply_positions.keys();
    for i in 0..deposit_keys.len() {
        let asset = deposit_keys.get(i).unwrap();
        if let Some(pos) = account.supply_positions.get(asset.clone()) {
            let feed = cache.cached_price(&asset);
            let market_index = cache.cached_market_index(&asset);
            let full_amount = Ray::from_raw(pos.scaled_amount_ray)
                .mul(env, Ray::from_raw(market_index.supply_index_ray))
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
                full_amount,
                &pos,
                false,
                0,
                feed.price_wad,
                cache,
            );
        }
    }
}

/// Pre-flight guard for swap_collateral: rejects isolated assets, deprecated e-mode,
/// non-collateralizable targets, and position limit violations before any token moves.
pub fn validate_swap_new_collateral_preflight(
    env: &Env,
    cache: &mut ControllerCache,
    account: &Account,
    new_collateral: &Address,
) {
    // Apply the e-mode category. Reject deprecated categories explicitly so
    // a user whose stored `loan_to_value_bps` reflects a now-retired e-mode
    // cap cannot ride the boosted parameters through the swap-collateral
    // path. `process_deposit` would also catch this later, but failing here
    // avoids running pool::withdraw + swap_tokens for a doomed transaction.
    let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);
    let config = emode::effective_asset_config(env, account, new_collateral, cache, &e_mode);
    if config.is_isolated_asset {
        // swap_collateral generally serves non-isolated positions only.
        // Isolated accounts use repayDebtWithCollateral to deleverage.
        panic_with_error!(env, EModeError::MixIsolatedCollateral);
    }
    emode::ensure_e_mode_compatible_with_asset(env, &config, account.e_mode_category_id);
    emode::validate_e_mode_asset(env, account.e_mode_category_id, new_collateral, true);

    if !config.is_collateralizable {
        panic_with_error!(env, CollateralError::NotCollateral);
    }

    // Extra pre-flight: check DEPOSIT position limits when the destination is a new asset.
    if !account
        .supply_positions
        .contains_key(new_collateral.clone())
    {
        let new_assets = soroban_sdk::vec![env, (new_collateral.clone(), 0i128)];
        validation::validate_bulk_position_limits(env, account, POSITION_TYPE_DEPOSIT, &new_assets);
    }
}
