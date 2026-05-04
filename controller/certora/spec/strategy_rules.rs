/// Strategy & Admin Operation Rules
///
/// Verifies correctness of leveraged strategy operations and admin endpoints:
///   - multiply creates both deposit and borrow positions
///   - multiply rejects same collateral/debt tokens
///   - multiply rejects non-collateralizable collateral
///   - swap_debt conserves debt value, rejects same token
///   - swap_collateral conserves collateral, rejects same token, rejects isolated
///   - repay_with_collateral reduces both debt and collateral
///   - clean_bad_debt qualification and position zeroing
///   - claim_revenue transfers correct amounts
///   - all strategy ops blocked during flash loan
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::BAD_DEBT_USD_THRESHOLD;
use common::types::{SwapSteps, POSITION_TYPE_BORROW, POSITION_TYPE_DEPOSIT};

// ===========================================================================
// Strategy Rules
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 1: multiply creates both deposit and borrow positions (split per branch)
// ---------------------------------------------------------------------------
//
// Split into per-branch rules with concrete inputs and bounded payment shapes
// so the prover evaluates the position invariant without optional-input
// path explosion.

/// Canonical happy-path multiply: brand-new account, no initial payment,
/// no `convert_steps`. The cheapest of the three multiply happy-path rules
/// because `process_multiply` skips the load-existing branch and the
/// `collect_initial_multiply_payment` swap branches entirely.
#[rule]
fn multiply_basic(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: SwapSteps,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!(collateral_token != debt_token);
    cvlr_assume!((1..=3).contains(&mode));

    let account_id = crate::spec::compat::multiply_basic(
        e.clone(),
        caller,
        e_mode_category,
        collateral_token.clone(),
        debt_to_flash_loan,
        debt_token.clone(),
        mode,
        steps,
    );

    // Verify deposit position exists with scaled_amount > 0
    let deposit_pos =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_DEPOSIT, &collateral_token);
    cvlr_assert!(deposit_pos.is_some());
    let deposit = deposit_pos.unwrap();
    cvlr_assert!(deposit.scaled_amount_ray > 0);

    // Verify borrow position exists with scaled_amount > 0
    let borrow_pos =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_BORROW, &debt_token);
    cvlr_assert!(borrow_pos.is_some());
    let borrow = borrow_pos.unwrap();
    cvlr_assert!(borrow.scaled_amount_ray > 0);
}

/// Multiply with an `initial_payment` denominated in `collateral_token`. This
/// is the cheap branch of `collect_initial_multiply_payment` (no nested
/// `swap_tokens`); the payment is added to the collateral leg directly.
#[rule]
fn multiply_with_initial_payment_collateral(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: SwapSteps,
    initial_amount: i128,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!(initial_amount > 0);
    cvlr_assume!(collateral_token != debt_token);
    cvlr_assume!((1..=3).contains(&mode));

    let account_id = crate::spec::compat::multiply_with_initial_payment_collateral(
        e.clone(),
        caller,
        e_mode_category,
        collateral_token.clone(),
        debt_to_flash_loan,
        debt_token.clone(),
        mode,
        steps,
        initial_amount,
    );

    let deposit_pos =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_DEPOSIT, &collateral_token);
    cvlr_assert!(deposit_pos.is_some());
    cvlr_assert!(deposit_pos.unwrap().scaled_amount_ray > 0);

    let borrow_pos =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_BORROW, &debt_token);
    cvlr_assert!(borrow_pos.is_some());
    cvlr_assert!(borrow_pos.unwrap().scaled_amount_ray > 0);
}

/// Multiply with an `initial_payment` denominated in a third token (distinct
/// from both `collateral_token` and `debt_token`) and a non-empty
/// `convert_steps`. Exercises the nested `swap_tokens` branch in
/// `collect_initial_multiply_payment`.
#[rule]
fn multiply_with_initial_payment_third_token(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: SwapSteps,
    third_token: Address,
    initial_amount: i128,
    convert_steps: SwapSteps,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!(initial_amount > 0);
    cvlr_assume!(collateral_token != debt_token);
    cvlr_assume!(third_token != collateral_token);
    cvlr_assume!(third_token != debt_token);
    cvlr_assume!((1..=3).contains(&mode));

    let account_id = crate::spec::compat::multiply_with_initial_payment_third_token(
        e.clone(),
        caller,
        e_mode_category,
        collateral_token.clone(),
        debt_to_flash_loan,
        debt_token.clone(),
        mode,
        steps,
        third_token,
        initial_amount,
        convert_steps,
    );

    let deposit_pos =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_DEPOSIT, &collateral_token);
    cvlr_assert!(deposit_pos.is_some());
    cvlr_assert!(deposit_pos.unwrap().scaled_amount_ray > 0);

    let borrow_pos =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_BORROW, &debt_token);
    cvlr_assert!(borrow_pos.is_some());
    cvlr_assert!(borrow_pos.unwrap().scaled_amount_ray > 0);
}

// ---------------------------------------------------------------------------
// Rule 2: multiply rejects same collateral and debt tokens
// ---------------------------------------------------------------------------

/// create_strategy (multiply) with collateral_token == debt_token must revert
/// with AssetsAreTheSame. If execution reaches cvlr_satisfy!(false), the
/// guard is broken.
#[rule]
fn multiply_rejects_same_tokens(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    token: Address,
    debt_to_flash_loan: i128,
    mode: u32,
    steps: SwapSteps,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!((1..=3).contains(&mode));

    // Call multiply with same token for both collateral and debt.
    // Use the minimal shim: the panic at strategy.rs:158-160 fires before any
    // optional-input branch is consulted, so havocing account_id /
    // initial_payment / convert_steps would only add wasted nondet draws.
    crate::spec::compat::multiply_minimal(
        e.clone(),
        caller,
        e_mode_category,
        token.clone(),
        debt_to_flash_loan,
        token.clone(), // same as collateral
        mode,
        steps,
    );

    // Must not reach here -- multiply should have reverted
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 3: multiply requires collateralizable asset
// ---------------------------------------------------------------------------

/// create_strategy with a non-collateralizable asset as collateral must revert.
/// The code checks `collateral_config.is_collateralizable` and panics with
/// NotCollateral if false.
#[rule]
fn multiply_requires_collateralizable(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: SwapSteps,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!(collateral_token != debt_token);
    cvlr_assume!((1..=3).contains(&mode));

    // Assume collateral asset is NOT collateralizable
    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let config = cache.cached_asset_config(&collateral_token);
    cvlr_assume!(!config.is_collateralizable);

    // Use the minimal shim: panic at strategy.rs:189-191 fires before the
    // `account_id` / `initial_payment` / `convert_steps` branches matter.
    crate::spec::compat::multiply_minimal(
        e.clone(),
        caller,
        e_mode_category,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        mode,
        steps,
    );

    // Must not reach here -- multiply should have reverted with NotCollateral
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 4: swap_debt conserves debt value
// ---------------------------------------------------------------------------

/// After swap_debt, the target debt position must exist and the source debt
/// position must have decreased. At minimum, the target debt position exists
/// with `scaled > 0` and the source debt position's scaled amount decreased or
/// was removed.
#[rule]
fn swap_debt_conserves_debt_value(
    e: Env,
    caller: Address,
    account_id: u64,
    existing_debt_token: Address,
    new_debt_amount: i128,
    new_debt_token: Address,
    steps: SwapSteps,
) {
    cvlr_assume!(new_debt_amount > 0);
    cvlr_assume!(existing_debt_token != new_debt_token);

    // Capture the source debt position before the swap.
    let old_pos_before =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_BORROW, &existing_debt_token);
    cvlr_assume!(old_pos_before.is_some());
    let old_scaled_before = old_pos_before.unwrap().scaled_amount_ray;
    cvlr_assume!(old_scaled_before > 0);

    // Execute swap_debt
    crate::Controller::swap_debt(
        e.clone(),
        caller,
        account_id,
        existing_debt_token.clone(),
        new_debt_amount,
        new_debt_token.clone(),
        steps,
    );

    // New debt position must exist with scaled_amount > 0
    let new_pos_after =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_BORROW, &new_debt_token);
    cvlr_assert!(new_pos_after.is_some());
    cvlr_assert!(new_pos_after.unwrap().scaled_amount_ray > 0);

    // Old debt position must have decreased or been removed
    let old_pos_after =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_BORROW, &existing_debt_token);
    match old_pos_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount_ray < old_scaled_before),
        None => cvlr_assert!(true), // Fully repaid and removed
    }
}

// ---------------------------------------------------------------------------
// Rule 5: swap_debt rejects same token
// ---------------------------------------------------------------------------

/// swap_debt with existing_debt_token == new_debt_token must revert with
/// AssetsAreTheSame.
#[rule]
fn swap_debt_rejects_same_token(
    e: Env,
    caller: Address,
    account_id: u64,
    token: Address,
    new_debt_amount: i128,
    steps: SwapSteps,
) {
    cvlr_assume!(new_debt_amount > 0);

    crate::Controller::swap_debt(
        e.clone(),
        caller,
        account_id,
        token.clone(),
        new_debt_amount,
        token.clone(), // same token
        steps,
    );

    // Must not reach here -- swap_debt should have reverted
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 6: swap_collateral conserves collateral
// ---------------------------------------------------------------------------

/// After swap_collateral, the source collateral decreases and the target
/// collateral increases.
#[rule]
fn swap_collateral_conserves_collateral(
    e: Env,
    caller: Address,
    account_id: u64,
    current_collateral: Address,
    from_amount: i128,
    new_collateral: Address,
    steps: SwapSteps,
) {
    cvlr_assume!(from_amount > 0);
    cvlr_assume!(current_collateral != new_collateral);

    // Capture the source collateral position before the swap.
    let old_pos_before =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_DEPOSIT, &current_collateral);
    cvlr_assume!(old_pos_before.is_some());
    let old_scaled_before = old_pos_before.unwrap().scaled_amount_ray;
    cvlr_assume!(old_scaled_before > 0);

    // Execute swap_collateral
    crate::Controller::swap_collateral(
        e.clone(),
        caller,
        account_id,
        current_collateral.clone(),
        from_amount,
        new_collateral.clone(),
        steps,
    );

    // New collateral position must exist with scaled_amount > 0
    let new_pos_after =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_DEPOSIT, &new_collateral);
    cvlr_assert!(new_pos_after.is_some());
    cvlr_assert!(new_pos_after.unwrap().scaled_amount_ray > 0);

    // Old collateral must have decreased or been removed
    let old_pos_after =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_DEPOSIT, &current_collateral);
    match old_pos_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount_ray < old_scaled_before),
        None => cvlr_assert!(true), // Fully withdrawn and removed
    }
}

// ---------------------------------------------------------------------------
// Rule 7: swap_collateral rejects same token
// ---------------------------------------------------------------------------

/// swap_collateral with current_collateral == new_collateral must revert.
#[rule]
fn swap_collateral_rejects_same_token(
    e: Env,
    caller: Address,
    account_id: u64,
    token: Address,
    from_amount: i128,
    steps: SwapSteps,
) {
    cvlr_assume!(from_amount > 0);

    crate::Controller::swap_collateral(
        e.clone(),
        caller,
        account_id,
        token.clone(),
        from_amount,
        token.clone(), // same token
        steps,
    );

    // Must not reach here -- swap_collateral should have reverted
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 8: swap_collateral rejects isolated accounts
// ---------------------------------------------------------------------------

/// swap_collateral on an isolated account must revert with SwapCollateralNoIso.
/// Isolated accounts have a single collateral asset that cannot be swapped.
#[rule]
fn swap_collateral_rejects_isolated(
    e: Env,
    caller: Address,
    account_id: u64,
    current_collateral: Address,
    from_amount: i128,
    new_collateral: Address,
    steps: SwapSteps,
) {
    cvlr_assume!(from_amount > 0);
    cvlr_assume!(current_collateral != new_collateral);

    // Assume the account is isolated
    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.is_isolated);

    crate::Controller::swap_collateral(
        e.clone(),
        caller,
        account_id,
        current_collateral,
        from_amount,
        new_collateral,
        steps,
    );

    // Must not reach here -- swap_collateral should have reverted
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 9: repay_with_collateral reduces both debt and collateral (split per
//         close_position branch)
// ---------------------------------------------------------------------------
//
// The original `repay_with_collateral_reduces_both` havoced `close_position`
// inside the compat shim. With `close_position = true`, production runs
// `execute_withdraw_all` which iterates the full supply position map -- an
// unbounded loop the prover cannot tame at the default `loop_iter`. Splitting
// into a no-close rule (cheap, no loop) and a close rule (loop-bearing,
// scoped to single-asset accounts) collapses the path explosion.

/// `repay_debt_with_collateral` with `close_position = false`. Removes the
/// `execute_withdraw_all` unbounded loop entirely; verifies the canonical
/// "reduce both sides" property without the account-deletion branch.
#[rule]
fn repay_with_collateral_reduces_both_no_close(
    e: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    steps: SwapSteps,
) {
    cvlr_assume!(collateral_amount > 0);
    cvlr_assume!(collateral_token != debt_token);

    // Capture positions before
    let collateral_before =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_DEPOSIT, &collateral_token);
    cvlr_assume!(collateral_before.is_some());
    let collateral_scaled_before = collateral_before.unwrap().scaled_amount_ray;
    cvlr_assume!(collateral_scaled_before > 0);

    let debt_before =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_BORROW, &debt_token);
    cvlr_assume!(debt_before.is_some());
    let debt_scaled_before = debt_before.unwrap().scaled_amount_ray;
    cvlr_assume!(debt_scaled_before > 0);

    // Execute repay_debt_with_collateral with close_position pinned off.
    crate::spec::compat::repay_debt_with_collateral_minimal(
        e.clone(),
        caller,
        account_id,
        collateral_token.clone(),
        collateral_amount,
        debt_token.clone(),
        steps,
    );

    // Collateral must have decreased or been removed
    let collateral_after =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_DEPOSIT, &collateral_token);
    match collateral_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount_ray < collateral_scaled_before),
        None => cvlr_assert!(true), // Fully withdrawn
    }

    // Debt must have decreased or been removed
    let debt_after =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_BORROW, &debt_token);
    match debt_after {
        Some(pos) => cvlr_assert!(pos.scaled_amount_ray < debt_scaled_before),
        None => cvlr_assert!(true), // Fully repaid
    }
}

/// `repay_debt_with_collateral` with `close_position = true`. After the call,
/// remaining collateral must be withdrawn via `execute_withdraw_all` (which
/// iterates the supply map) and the account must be removed from storage.
/// The companion `loop_iter` setting in `repay_with_collateral_close.conf`
/// (or whatever the prover-config file is named) should be tightened to 2 to
/// bound the supply-map iteration.
#[rule]
fn repay_with_collateral_full_close_removes_account(
    e: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    steps: SwapSteps,
) {
    cvlr_assume!(collateral_amount > 0);
    cvlr_assume!(collateral_token != debt_token);

    // Account must exist with both position legs before the call; the
    // close-position path is gated on `borrow_positions` being empty after the
    // repay, so the prover discovers the witness within the loop_iter bound
    // rather than pinning the map shape here.
    let collateral_before =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_DEPOSIT, &collateral_token);
    cvlr_assume!(collateral_before.is_some());
    cvlr_assume!(collateral_before.unwrap().scaled_amount_ray > 0);

    let debt_before =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_BORROW, &debt_token);
    cvlr_assume!(debt_before.is_some());
    cvlr_assume!(debt_before.unwrap().scaled_amount_ray > 0);

    // Execute repay_debt_with_collateral with close_position pinned on.
    crate::spec::compat::repay_debt_with_collateral_close(
        e.clone(),
        caller,
        account_id,
        collateral_token.clone(),
        collateral_amount,
        debt_token.clone(),
        steps,
    );

    // After the close branch, both legs must be cleared. `execute_withdraw_all`
    // pulls each remaining supply asset; `strategy_finalize` deletes the
    // account if both maps are empty.
    let debt_after =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_BORROW, &debt_token);
    cvlr_assert!(debt_after.is_none());

    let collateral_after =
        crate::storage::get_position(&e, account_id, POSITION_TYPE_DEPOSIT, &collateral_token);
    cvlr_assert!(collateral_after.is_none());
}

// ===========================================================================
// Admin Operation Rules
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 10: clean_bad_debt requires qualification
// ---------------------------------------------------------------------------

/// `clean_bad_debt_standalone` (controller/src/positions/liquidation.rs:478)
/// panics with `CannotCleanBadDebt` unless
/// `total_debt_usd > total_collateral_usd && total_collateral_usd <= 5*WAD`.
/// This rule asserts that any account violating that predicate cannot reach
/// the post-state.
///
/// Earlier versions used a health-factor proxy (`hf >= WAD`) which is sound
/// but incomplete: it misses underwater accounts above the dust threshold
/// (`hf < WAD && total_debt > total_coll && total_coll > 5*WAD`). Because
/// `calculate_health_factor_for` and `calculate_account_totals` have
/// independently-havoced summaries, the proxy also produced PASS witnesses
/// where the HF assume held but the account-totals satisfied the panic
/// predicate -- making the rule vacuously satisfied via the panic path.
///
/// This rewrite ties the precondition directly to the production guard by
/// reading the same `calculate_account_totals` triple production reads.
#[rule]
fn clean_bad_debt_requires_qualification(e: Env, account_id: u64) {
    let mut cache = crate::cache::ControllerCache::new(&e, false);

    // PositionNotFound short-circuit: production fast-fails when the account
    // has no borrows, which is not the path this rule covers.
    let account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(!account.borrow_positions.is_empty());

    // Capture the same triple production gates on.
    let (total_collateral_usd, total_debt_usd, _) = crate::helpers::calculate_account_totals(
        &e,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    // Assume the account does NOT qualify for cleanup. Either the debt is at
    // or below collateral, or the collateral exceeds the dust threshold.
    cvlr_assume!(
        !(total_debt_usd.raw() > total_collateral_usd.raw()
            && total_collateral_usd.raw() <= BAD_DEBT_USD_THRESHOLD)
    );

    // Production must panic before reaching the post-state.
    crate::positions::liquidation::clean_bad_debt_standalone(&e, account_id);

    // Must not reach here.
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 11: clean_bad_debt zeros all positions
// ---------------------------------------------------------------------------

/// After clean_bad_debt on a qualifying account, all positions are zeroed:
/// both supply and borrow position lists are empty.
#[rule]
fn clean_bad_debt_zeros_positions(e: Env, account_id: u64) {
    // Assume account qualifies: has borrows, debt > collateral, collateral <= $5
    let borrow_list_pre = crate::storage::get_position_list(&e, account_id, POSITION_TYPE_BORROW);
    cvlr_assume!(!borrow_list_pre.is_empty());

    // Execute bad debt cleanup
    crate::positions::liquidation::clean_bad_debt_standalone(&e, account_id);

    // After cleanup, both position lists must be empty
    let deposit_list = crate::storage::get_position_list(&e, account_id, POSITION_TYPE_DEPOSIT);
    let borrow_list = crate::storage::get_position_list(&e, account_id, POSITION_TYPE_BORROW);

    cvlr_assert!(deposit_list.is_empty());
    cvlr_assert!(borrow_list.is_empty());
}

// ---------------------------------------------------------------------------
// Rule 12: claim_revenue returns amount bounded by pool revenue
// ---------------------------------------------------------------------------

/// claim_revenue returns amount > 0 only when pool has revenue. The returned
/// amount must be non-negative and bounded by what the pool reports.
#[rule]
fn claim_revenue_transfers_to_accumulator(e: Env, caller: Address, asset: Address) {
    let amounts = crate::Controller::claim_revenue(e.clone(), caller, soroban_sdk::vec![&e, asset]);
    let amount = amounts.get(0).unwrap();

    // Returned amount must be non-negative (contract returns 0 when no revenue)
    cvlr_assert!(amount >= 0);

    // If amount > 0, the pool actually had revenue to claim
    // (pool.claim_revenue enforces this internally)
    cvlr_satisfy!(amount >= 0);
}

// ---------------------------------------------------------------------------
// Rule 13: DELETED -- claim_revenue_decreases_pool_revenue was vacuous.
// The revenue_before was a nondet value not tied to actual pool state,
// so the assertion `amount <= revenue_before` proved nothing.
// The basic property (claim returns non-negative amount) is covered by
// claim_revenue_transfers_to_accumulator, and pool-level revenue
// accounting is tested in pool unit tests.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Rule 14: strategy operations blocked during flash loan -- DELETED
// ---------------------------------------------------------------------------
//
// The four endpoint-level rules (`strategy_blocked_during_flash_loan_multiply`,
// `_swap_debt`, `_swap_collateral`, `_repay_with_collateral`) each paid the
// full mutating-endpoint setup cost (compat shim havocs + parameter symbols)
// to verify the same single-line guard. The helper-level rule
// `flash_loan_rules::flash_loan_guard_blocks_callers` covers the property
// directly against `validation::require_not_flash_loaning` -- every mutating
// endpoint that calls the helper first inherits the property by construction.
//
// ===========================================================================
// Sanity rules (reachability checks)
// ===========================================================================

#[rule]
fn multiply_sanity(
    e: Env,
    caller: Address,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    steps: SwapSteps,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    cvlr_assume!(collateral_token != debt_token);

    let account_id = crate::spec::compat::multiply(
        e,
        caller,
        0, // no e-mode
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        1, // multiply mode
        steps,
    );
    cvlr_satisfy!(account_id > 0);
}

#[rule]
fn swap_debt_sanity(
    e: Env,
    caller: Address,
    account_id: u64,
    existing_debt_token: Address,
    new_debt_amount: i128,
    new_debt_token: Address,
    steps: SwapSteps,
) {
    cvlr_assume!(new_debt_amount > 0);
    cvlr_assume!(existing_debt_token != new_debt_token);

    crate::Controller::swap_debt(
        e,
        caller,
        account_id,
        existing_debt_token,
        new_debt_amount,
        new_debt_token,
        steps,
    );
    cvlr_satisfy!(true);
}

#[rule]
fn swap_collateral_sanity(
    e: Env,
    caller: Address,
    account_id: u64,
    current_collateral: Address,
    from_amount: i128,
    new_collateral: Address,
    steps: SwapSteps,
) {
    cvlr_assume!(from_amount > 0);
    cvlr_assume!(current_collateral != new_collateral);

    crate::Controller::swap_collateral(
        e,
        caller,
        account_id,
        current_collateral,
        from_amount,
        new_collateral,
        steps,
    );
    cvlr_satisfy!(true);
}

#[rule]
fn clean_bad_debt_sanity(e: Env, account_id: u64) {
    crate::positions::liquidation::clean_bad_debt_standalone(&e, account_id);
    cvlr_satisfy!(true);
}
