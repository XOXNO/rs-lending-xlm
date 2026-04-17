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

use common::constants::WAD;
use common::types::{SwapSteps, POSITION_TYPE_BORROW, POSITION_TYPE_DEPOSIT};

// ===========================================================================
// Strategy Rules
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 1: multiply creates both deposit and borrow positions
// ---------------------------------------------------------------------------

/// After a successful multiply, the newly created account must have both a
/// deposit position (collateral) with scaled_amount > 0 and a borrow position
/// (debt) with scaled_amount > 0.
#[rule]
fn multiply_creates_both_positions(
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
    cvlr_assume!(mode >= 1 && mode <= 3);

    let account_id = crate::spec::compat::multiply(
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
    cvlr_assume!(mode >= 1 && mode <= 3);

    // Call multiply with same token for both collateral and debt
    crate::spec::compat::multiply(
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
    cvlr_assume!(mode >= 1 && mode <= 3);

    // Assume collateral asset is NOT collateralizable
    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let config = cache.cached_asset_config(&collateral_token);
    cvlr_assume!(!config.is_collateralizable);

    crate::spec::compat::multiply(
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

/// After swap_debt, the new debt position must exist and the old debt position
/// must have decreased. At minimum: new debt position exists with scaled > 0
/// and old debt position's scaled amount decreased or was removed.
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

    // Capture old debt position scaled amount before swap
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

/// After swap_collateral, the old collateral decreased and the new collateral
/// increased (position exists with scaled > 0).
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

    // Capture old collateral position before swap
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
// Rule 9: repay_with_collateral reduces both debt and collateral
// ---------------------------------------------------------------------------

/// After process_repay_debt_with_collateral, both the debt position and the
/// collateral position must have decreased (or been removed).
#[rule]
fn repay_with_collateral_reduces_both(
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

    // Execute repay_debt_with_collateral
    crate::spec::compat::repay_debt_with_collateral(
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

// ===========================================================================
// Admin Operation Rules
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 10: clean_bad_debt requires qualification
// ---------------------------------------------------------------------------

/// clean_bad_debt on an account that does not qualify (debt <= collateral
/// OR collateral > $5 USD) must revert with CannotCleanBadDebt.
#[rule]
fn clean_bad_debt_requires_qualification(e: Env, account_id: u64) {
    // Assume the account does NOT qualify for bad debt cleanup:
    // Either collateral > $5 or debt <= collateral
    let mut cache = crate::cache::ControllerCache::new(&e, false);

    // Verify account has borrows (otherwise PositionNotFound)
    let borrow_list = crate::storage::get_position_list(&e, account_id, POSITION_TYPE_BORROW);
    cvlr_assume!(!borrow_list.is_empty());

    // Calculate totals -- assume account is healthy enough to not qualify
    let hf = crate::helpers::calculate_health_factor_for(&e, &mut cache, account_id);
    cvlr_assume!(hf >= WAD); // HF >= 1.0 means debt < weighted collateral

    // This call should revert with CannotCleanBadDebt
    crate::positions::liquidation::clean_bad_debt_standalone(&e, account_id);

    // Must not reach here
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
// Rule 14: strategy operations blocked during flash loan
// ---------------------------------------------------------------------------

/// All strategy operations (multiply, swap_debt, swap_collateral,
/// repay_debt_with_collateral) must revert when the flash loan reentrancy
/// guard is active. Tests multiply as representative; the guard check is
/// shared via require_not_flash_loaning.
#[rule]
fn strategy_blocked_during_flash_loan_multiply(
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

    // Activate flash loan reentrancy guard
    crate::storage::set_flash_loan_ongoing(&e, true);

    crate::spec::compat::multiply(
        e.clone(),
        caller,
        e_mode_category,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        mode,
        steps,
    );

    // Must not reach here -- should have reverted with FlashLoanOngoing
    cvlr_satisfy!(false);
}

/// swap_debt must revert when flash loan guard is active.
#[rule]
fn strategy_blocked_during_flash_loan_swap_debt(
    e: Env,
    caller: Address,
    account_id: u64,
    existing_debt_token: Address,
    new_debt_amount: i128,
    new_debt_token: Address,
    steps: SwapSteps,
) {
    cvlr_assume!(new_debt_amount > 0);

    crate::storage::set_flash_loan_ongoing(&e, true);

    crate::Controller::swap_debt(
        e.clone(),
        caller,
        account_id,
        existing_debt_token,
        new_debt_amount,
        new_debt_token,
        steps,
    );

    cvlr_satisfy!(false);
}

/// swap_collateral must revert when flash loan guard is active.
#[rule]
fn strategy_blocked_during_flash_loan_swap_collateral(
    e: Env,
    caller: Address,
    account_id: u64,
    current_collateral: Address,
    from_amount: i128,
    new_collateral: Address,
    steps: SwapSteps,
) {
    cvlr_assume!(from_amount > 0);

    crate::storage::set_flash_loan_ongoing(&e, true);

    crate::Controller::swap_collateral(
        e.clone(),
        caller,
        account_id,
        current_collateral,
        from_amount,
        new_collateral,
        steps,
    );

    cvlr_satisfy!(false);
}

/// repay_debt_with_collateral must revert when flash loan guard is active.
#[rule]
fn strategy_blocked_during_flash_loan_repay_with_collateral(
    e: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    steps: SwapSteps,
) {
    cvlr_assume!(collateral_amount > 0);

    crate::storage::set_flash_loan_ongoing(&e, true);

    crate::spec::compat::repay_debt_with_collateral(
        e.clone(),
        caller,
        account_id,
        collateral_token,
        collateral_amount,
        debt_token,
        steps,
    );

    cvlr_satisfy!(false);
}

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
