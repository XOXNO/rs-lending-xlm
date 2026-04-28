use common::constants::WAD;
use common::errors::{CollateralError, EModeError, FlashLoanError, GenericError, StrategyError};
use common::events::{emit_initial_multiply_payment, InitialMultiplyPaymentEvent};
use common::fp::{Ray, Wad};
use common::types::{
    Account, PositionMode, SwapSteps, POSITION_TYPE_BORROW, POSITION_TYPE_DEPOSIT,
};
use soroban_sdk::{panic_with_error, symbol_short, Address, Env, Vec};

use crate::cache::ControllerCache;
use crate::{
    helpers,
    positions::{borrow, emode, repay, supply, withdraw},
    storage, utils, validation,
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
    validation::require_not_paused(env);
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

    let mut collateral_amount: i128 = 0;
    let mut debt_extra: i128 = 0;
    if let Some((payment_token, payment_amount)) = &initial_payment {
        validation::require_amount_positive(env, *payment_amount);

        let payment_tok = soroban_sdk::token::Client::new(env, payment_token);
        payment_tok.transfer(caller, env.current_contract_address(), payment_amount);

        if *payment_token == *collateral_token {
            // Payment is the collateral token; credit it directly.
            collateral_amount = *payment_amount;
        } else if *payment_token == *debt_token {
            // Payment is the debt token; add to the flash-loan swap input
            // to increase the leveraged collateral output.
            debt_extra = *payment_amount;
        } else {
            // Third token; route through convert_steps to the collateral.
            let convert = match convert_steps {
                Some(steps) => steps,
                None => panic_with_error!(env, StrategyError::ConvertStepsRequired),
            };
            collateral_amount = swap_tokens(
                env,
                payment_token,
                *payment_amount,
                collateral_token,
                &convert,
                caller,
            );
        }
    }

    let is_new_account = account_id == 0;

    // Strict-price cache: strategy borrows are risk-increasing.
    let mut cache = ControllerCache::new(env, false);

    let collateral_config = cache.cached_asset_config(collateral_token);
    if !collateral_config.is_collateralizable {
        panic_with_error!(env, CollateralError::NotCollateral);
    }

    let (account_id, mut account) = if is_new_account {
        let is_isolated = collateral_config.is_isolated_asset;
        let isolated_asset = if is_isolated {
            Some(collateral_token.clone())
        } else {
            None
        };
        let new_id = utils::create_account(
            env,
            caller,
            e_mode_category,
            mode,
            is_isolated,
            isolated_asset,
        );
        (new_id, storage::get_account(env, new_id))
    } else {
        let existing = storage::get_account(env, account_id);
        // Caller was authenticated at entry; an owner check here avoids a
        // second `require_auth` invocation.
        if existing.owner != *caller {
            panic_with_error!(env, GenericError::AccountNotInMarket);
        }
        if existing.mode != mode {
            panic_with_error!(env, GenericError::AccountModeMismatch);
        }
        (account_id, existing)
    };

    // Borrow-position cap: enforce upfront before opening, mirroring
    // `borrow_batch`. The strategy passes a single-asset vec so the dedup
    // logic in validate_bulk_position_limits short-circuits cleanly.
    let new_borrow_assets = soroban_sdk::vec![env, (debt_token.clone(), debt_to_flash_loan)];
    validation::validate_bulk_position_limits(
        env,
        &account,
        POSITION_TYPE_BORROW,
        &new_borrow_assets,
    );

    // Validates e-mode, borrowability, siloed rules, borrow cap, and
    // isolated-debt ceiling, flashes the debt via `pool.create_strategy`,
    // and returns the net amount received.
    let mut debt_config = cache.cached_asset_config(debt_token);
    let amount_received = borrow::handle_create_borrow_strategy(
        env,
        &mut cache,
        &mut account,
        account_id,
        debt_token,
        debt_to_flash_loan,
        &mut debt_config,
        caller,
    );

    // Include any debt-token initial payment in the swap input.
    let swapped_collateral = swap_tokens(
        env,
        debt_token,
        amount_received + debt_extra,
        collateral_token,
        steps,
        caller,
    );

    let total_collateral = collateral_amount + swapped_collateral;

    // Deposit pipeline applies e-mode, supply caps, risk parameters.
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
    validation::require_not_paused(env);
    validation::require_not_flash_loaning(env);

    if existing_debt_token == new_debt_token {
        panic_with_error!(env, GenericError::AssetsAreTheSame);
    }

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner(env, &account, caller);

    // Strict-price cache: strategy borrows are risk-increasing.
    let mut cache = ControllerCache::new(env, false);

    validation::require_amount_positive(env, new_debt_amount);
    // Reject zero-floor swap requests at entry.
    validation::require_amount_positive(env, steps.amount_out_min);

    // Reject swap_debt when either side is siloed: the flow holds both debt
    // positions simultaneously (new is borrowed before old is repaid),
    // which violates the siloed-borrow invariant.
    let existing_debt_config = cache.cached_asset_config(existing_debt_token);
    let mut new_debt_config = cache.cached_asset_config(new_debt_token);
    let asset_emode_config =
        emode::token_e_mode_config(env, account.e_mode_category_id, new_debt_token);
    emode::ensure_e_mode_compatible_with_asset(env, &new_debt_config, account.e_mode_category_id);
    let e_mode = emode::e_mode_category(env, account.e_mode_category_id);
    emode::apply_e_mode_to_asset_config(env, &mut new_debt_config, &e_mode, asset_emode_config);
    if existing_debt_config.is_siloed_borrowing || new_debt_config.is_siloed_borrowing {
        panic_with_error!(env, CollateralError::NotBorrowableSiloed);
    }

    // Borrow-position cap: enforce upfront before opening the new debt,
    // mirroring `borrow_batch`.
    let new_borrow_assets = soroban_sdk::vec![env, (new_debt_token.clone(), new_debt_amount)];
    validation::validate_bulk_position_limits(
        env,
        &account,
        POSITION_TYPE_BORROW,
        &new_borrow_assets,
    );

    // Flashes the new debt via `pool.create_strategy` after the standard
    // e-mode, borrowability, siloed, borrow-cap, and isolated-debt-ceiling
    // checks; returns the net amount received.
    let amount_received = borrow::handle_create_borrow_strategy(
        env,
        &mut cache,
        &mut account,
        account_id,
        new_debt_token,
        new_debt_amount,
        &mut new_debt_config,
        caller,
    );

    // Use the net amount after flash-loan fee as swap input.
    let swapped_amount = swap_tokens(
        env,
        new_debt_token,
        amount_received,
        existing_debt_token,
        steps,
        caller,
    );

    let existing_pool_addr = cache.cached_pool_address(existing_debt_token);
    let existing_feed = cache.cached_price(existing_debt_token);

    let existing_pos = account
        .borrow_positions
        .get(existing_debt_token.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound));

    let existing_tok = soroban_sdk::token::Client::new(env, existing_debt_token);
    existing_tok.transfer(
        &env.current_contract_address(),
        &existing_pool_addr,
        &swapped_amount,
    );

    let controller_balance_before_repay = existing_tok.balance(&env.current_contract_address());

    // Shared repay path: pool.repay + position update + isolated-debt
    // adjustment + UpdatePositionEvent with `sw_debt_r` action tag.
    // `event_caller = caller` (original user) so indexers see the real
    // initiator, not the controller's own address.
    repay::execute_repayment(
        env,
        &mut account,
        &env.current_contract_address(),
        caller,
        symbol_short!("sw_debt_r"),
        &existing_pos,
        existing_feed.price_wad,
        swapped_amount,
        &mut cache,
    );

    // Pool.repay refunds overpayment to the controller; forward to caller.
    let controller_balance_after_repay = existing_tok.balance(&env.current_contract_address());
    let repay_excess =
        controller_balance_after_repay.saturating_sub(controller_balance_before_repay);
    if repay_excess > 0 {
        existing_tok.transfer(&env.current_contract_address(), caller, &repay_excess);
    }

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
    validation::require_not_paused(env);
    validation::require_not_flash_loaning(env);

    if current_collateral == new_collateral {
        panic_with_error!(env, GenericError::AssetsAreTheSame);
    }

    let mut account = storage::get_account(env, account_id);
    validation::require_account_owner(env, &account, caller);

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

    let current_feed = cache.cached_price(current_collateral);

    let current_pos = account
        .supply_positions
        .get(current_collateral.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::CollateralPositionNotFound));

    // Snapshot the controller's balance before withdrawal. Pools may
    // deliver slightly less than requested (rounding, dust floors, reserve
    // caps); using the requested figure as the swap input would leak dust
    // or over-approve the aggregator.
    let current_tok_client = soroban_sdk::token::Client::new(env, current_collateral);
    let controller_balance_before_withdraw =
        current_tok_client.balance(&env.current_contract_address());

    // Emits UpdatePositionEvent with `sw_col_wd` action tag + real user as caller.
    let _updated_current = withdraw::execute_withdrawal(
        env,
        account_id,
        &mut account,
        &env.current_contract_address(),
        caller,
        symbol_short!("sw_col_wd"),
        from_amount,
        &current_pos,
        false, // is_liquidation
        0,     // protocol_fee
        current_feed.price_wad,
        &mut cache,
    );

    let controller_balance_after_withdraw =
        current_tok_client.balance(&env.current_contract_address());
    let actual_withdrawn = controller_balance_after_withdraw
        .checked_sub(controller_balance_before_withdraw)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));

    let swapped_amount = swap_tokens(
        env,
        current_collateral,
        actual_withdrawn,
        new_collateral,
        steps,
        caller,
    );

    // Deposit pipeline applies e-mode, supply caps, and risk parameters.
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
    let balance_in_before = token_in_client.balance(&env.current_contract_address());
    let balance_out_before = token_out_client.balance(&env.current_contract_address());

    // Approve the router to pull token_in from the controller.
    token_in_client.approve(
        &env.current_contract_address(),
        &router_addr,
        &amount_in,
        &(env.ledger().sequence() + 200),
    );

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
        &(env.ledger().timestamp() + 3600),
    );

    storage::set_flash_loan_ongoing(env, false);

    // Verify the exact input spend. A well-behaved router pulls at most
    // `amount_in` and never returns tokens (balance going UP). Either
    // direction of misbehavior is an internal error.
    let balance_in_after = token_in_client.balance(&env.current_contract_address());
    if balance_in_after > balance_in_before {
        panic_with_error!(env, GenericError::InternalError);
    }
    let actual_in_spent = balance_in_before - balance_in_after;
    if actual_in_spent > amount_in {
        panic_with_error!(env, GenericError::InternalError);
    }

    // Zero any residual allowance so a compromised or lazy router cannot
    // pull additional funds after the swap returns.
    token_in_client.approve(&env.current_contract_address(), &router_addr, &0, &0);

    // Refund any unspent `token_in` to `refund_to` (= original user). A
    // router that partial-fills (route exhaustion, integer rounding, or a
    // bug) leaves residual `token_in` on the controller; the SEP-41
    // approve+pull model provides no callback-returned transfer bundle, so
    // unspent input requires manual reconciliation.
    let unspent = amount_in - actual_in_spent;
    if unspent > 0 {
        token_in_client.transfer(&env.current_contract_address(), refund_to, &unspent);
    }

    // Received must be non-negative. A decrease is impossible from a sane
    // token contract and indicates aggregator or token misbehavior.
    let balance_out_after = token_out_client.balance(&env.current_contract_address());
    let received = balance_out_after
        .checked_sub(balance_out_before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));

    // Enforce the slippage minimum at the controller so a router that
    // ignores its own `amount_out_min` cannot silently shortchange the
    // caller. Strategy entrypoints already reject `amount_out_min <= 0`.
    if received < steps.amount_out_min {
        panic_with_error!(env, GenericError::InternalError);
    }

    // Note: any third-party token the router happens to deposit into the
    // controller (LP rebate, governance reward, malformed output) is NOT
    // swept here. With SEP-41 push-on-transfer semantics only the
    // configured router can land tokens, and only `token_in` and
    // `token_out` are part of this swap's contract. Adding a generic sweep
    // would require an oracle of "expected output tokens" which the
    // strategy callsites already supply via `token_out`. If a future
    // aggregator integration emits multi-token output, expand the
    // signature with a `&Vec<Address>` of expected-zero-delta tokens.
    received
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
    validation::require_not_paused(env);
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
    validation::require_account_owner(env, &account, caller);

    let mut cache = ControllerCache::new(env, false);

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

    let collateral_feed = cache.cached_price(collateral_token);

    // Snapshot the controller balance before withdrawal so the swap sees
    // the amount actually received. Pools may deliver slightly less than
    // requested due to rounding or dust floors.
    let collateral_tok_client = soroban_sdk::token::Client::new(env, collateral_token);
    let controller_balance_before_withdraw =
        collateral_tok_client.balance(&env.current_contract_address());

    // Emits UpdatePositionEvent with `rp_col_wd` action tag.
    let _updated_collateral = withdraw::execute_withdrawal(
        env,
        account_id,
        &mut account,
        &env.current_contract_address(),
        caller,
        symbol_short!("rp_col_wd"),
        collateral_amount,
        &collateral_pos,
        false, // not liquidation
        0,     // no protocol fee
        collateral_feed.price_wad,
        &mut cache,
    );

    let controller_balance_after_withdraw =
        collateral_tok_client.balance(&env.current_contract_address());
    let actual_withdrawn = controller_balance_after_withdraw
        .checked_sub(controller_balance_before_withdraw)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));

    // Same-asset short-circuit: skip the router when collateral == debt.
    let swapped_debt = if collateral_token == debt_token {
        actual_withdrawn
    } else {
        swap_tokens(
            env,
            collateral_token,
            actual_withdrawn,
            debt_token,
            steps,
            caller,
        )
    };

    let debt_pool_addr = cache.cached_pool_address(debt_token);
    let debt_feed = cache.cached_price(debt_token);
    let debt_tok = soroban_sdk::token::Client::new(env, debt_token);

    // Pool-balance delta accounting around the transfer mirrors plain
    // `process_single_repay`: pass the amount that actually arrived to
    // `pool::repay`, not the requested `swapped_debt`. Defends against any
    // future onboarding of a fee-on-transfer or rebasing debt token where
    // `swapped_debt - fee` reaches the pool. Without the delta, the user's
    // debt position would be credited for the requested amount while the
    // pool's reserve diverges from its scaled-borrow accounting by the fee.
    let pool_balance_before_repay = debt_tok.balance(&debt_pool_addr);
    debt_tok.transfer(
        &env.current_contract_address(),
        &debt_pool_addr,
        &swapped_debt,
    );
    let pool_balance_after_repay = debt_tok.balance(&debt_pool_addr);
    let actual_arrived_at_pool = pool_balance_after_repay
        .checked_sub(pool_balance_before_repay)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    if actual_arrived_at_pool <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }

    let controller_balance_before_repay = debt_tok.balance(&env.current_contract_address());

    // Route through the shared repay path for isolated debt handling.
    // Emits UpdatePositionEvent with `rp_col_r` action tag.
    repay::execute_repayment(
        env,
        &mut account,
        &env.current_contract_address(),
        caller,
        symbol_short!("rp_col_r"),
        &debt_pos,
        debt_feed.price_wad,
        actual_arrived_at_pool,
        &mut cache,
    );

    // Refund excess to the caller.
    let controller_balance_after_repay = debt_tok.balance(&env.current_contract_address());
    let repay_excess =
        controller_balance_after_repay.saturating_sub(controller_balance_before_repay);
    if repay_excess > 0 {
        debt_tok.transfer(&env.current_contract_address(), caller, &repay_excess);
    }

    // Close-position flag withdraws all remaining collateral to the caller.
    // The HF gate runs in `strategy_finalize` (single source of truth);
    // having a second check here was dead defense-in-depth that read the
    // same prices and traversed the same maps.
    if close_position {
        if !account.borrow_positions.is_empty() {
            panic_with_error!(env, CollateralError::CannotCloseWithRemainingDebt);
        }

        execute_withdraw_all(env, account_id, &mut account, caller, &mut cache);
    }

    strategy_finalize(env, account_id, &mut account, &mut cache);
}

// ---------------------------------------------------------------------------
// Strategy Helpers
// ---------------------------------------------------------------------------

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
        storage::set_account(env, account_id, account);
    }

    // Re-check HF with a fresh price cache after the leveraged mutation.
    cache.clean_prices_cache();
    if !account.borrow_positions.is_empty() {
        let hf = helpers::calculate_health_factor(
            env,
            cache,
            &account.supply_positions,
            &account.borrow_positions,
        );
        if hf < WAD {
            panic_with_error!(env, CollateralError::InsufficientCollateral);
        }
    }

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
    account_id: u64,
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
            // Emits UpdatePositionEvent with `close_wd` action tag. `destination`
            // is the user here (strategy close-position withdraws to them), so
            // it doubles as the event caller.
            let _updated = withdraw::execute_withdrawal(
                env,
                account_id,
                account,
                destination,
                destination,
                symbol_short!("close_wd"),
                full_amount,
                &pos,
                false, // is_liquidation
                0,     // protocol_fee
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
    let mut config = cache.cached_asset_config(new_collateral);
    if config.is_isolated_asset {
        // swap_collateral generally serves non-isolated positions only.
        // Isolated accounts use repayDebtWithCollateral to deleverage.
        panic_with_error!(env, EModeError::MixIsolatedCollateral);
    }

    // Apply the e-mode category. Reject deprecated categories explicitly so
    // a user whose stored `loan_to_value_bps` reflects a now-retired e-mode
    // cap cannot ride the boosted parameters through the swap-collateral
    // path. `process_deposit` would also catch this later, but failing here
    // avoids running pool::withdraw + swap_tokens for a doomed transaction.
    let e_mode = emode::e_mode_category(env, account.e_mode_category_id);
    emode::ensure_e_mode_not_deprecated(env, &e_mode);
    let asset_emode_config = cache.cached_emode_asset(account.e_mode_category_id, new_collateral);
    emode::ensure_e_mode_compatible_with_asset(env, &config, account.e_mode_category_id);
    emode::apply_e_mode_to_asset_config(env, &mut config, &e_mode, asset_emode_config);
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
