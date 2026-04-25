/// Flash Loan Safety Rules
///
/// From CLAUDE.md:
///   - Reentrancy guard: no nested flash loans (FLASH_LOAN_ALREADY_ONGOING)
///   - repayment >= borrowed + fees enforced after callback
///   - Cache dropped before callback, recreated after (reentrancy protection)
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Bytes, Env};

// ---------------------------------------------------------------------------
// Rule 1: DELETED -- flash_loan_guard_active_during_callback was vacuous.
// It assumed the guard was true then asserted it was true (tautology).
// The guard behavior is properly tested by no_mutations_during_flash_loan
// which sets the guard in storage and attempts a borrow.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Rule 2: Flash loan repayment covers borrowed + fee
// ---------------------------------------------------------------------------

/// The pool must receive back at least (borrowed_amount + fee) after the
/// flash loan callback completes. This is enforced in pool.flash_loan_end().
///
/// Note: This property is partially verified here and partially in the pool
/// contract specs. The controller ensures the call sequence is correct.
// P1 rewrite: assert a pre/post revenue delta on the pool, not cvlr_satisfy!(true).
// If flash_loan completes successfully, the pool's revenue MUST not regress.
// A `>=` (not `>`) bound is the strongest correct assertion:
//   * Operators may set `flashloan_fee_bps == 0` (production accepts any fee in
//     `[0, MAX_FLASHLOAN_FEE_BPS]` -- see `validation::validate_asset_config`).
//   * Even with a positive fee, half-up rounding zeroes out tiny `amount`
//     values (`fee = amount * bps / BPS` rounds to 0 when
//     `amount * bps < BPS / 2`).
//   * `add_protocol_revenue` short-circuits when `supply_index <
//     SUPPLY_INDEX_FLOOR_RAW`, leaving revenue unchanged even on a
//     non-zero fee in pathological post-bad-debt states.
// In each of these cases the strict `>` form would fail despite production
// behaving correctly. The relaxed `>=` still catches the failure mode the
// rule was written to detect: a broken `flash_loan_end` path that *negatively*
// adjusts revenue.
#[rule]
fn flash_loan_fee_collected(
    e: Env,
    caller: Address,
    receiver: Address,
    asset: Address,
    amount: i128,
    data: Bytes,
) {
    cvlr_assume!(amount > 0);

    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let pool_addr = cache.cached_pool_address(&asset);

    let pool_client = pool_interface::LiquidityPoolClient::new(&e, &pool_addr);
    let revenue_before = pool_client.protocol_revenue();

    crate::Controller::flash_loan(e.clone(), caller, asset.clone(), amount, receiver, data);

    let revenue_after = pool_client.protocol_revenue();

    cvlr_assert!(revenue_after >= revenue_before);
}

// ---------------------------------------------------------------------------
// Rule 3: No state mutation possible during flash loan callback
// ---------------------------------------------------------------------------

/// During a flash loan callback, all mutating endpoints on the controller
/// must be blocked by the reentrancy guard.
///
/// Pattern: Set FlashLoanOngoing = true in storage, then attempt to call
/// Controller::borrow. If the call reverts (as expected), execution never
/// reaches cvlr_satisfy!(false). If the prover can reach it, the guard
/// is broken.
#[rule]
fn no_mutations_during_flash_loan(e: Env, caller: Address, account_id: u64, asset: Address) {
    // Activate the flash loan reentrancy guard in storage
    crate::storage::set_flash_loan_ongoing(&e, true);

    let amount: i128 = cvlr::nondet::nondet();
    cvlr_assume!(amount > 0);

    // Attempt a borrow while the flash loan guard is active.
    // This must revert with FlashLoanOngoing.
    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    // If execution reaches here, borrow succeeded despite the guard -- violation.
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 4: Flash loan guard cleared after successful completion
// ---------------------------------------------------------------------------

/// After a successful flash loan (process_flash_loan returns without
/// reverting), the FlashLoanOngoing guard must be reset to false.
/// This ensures subsequent operations are not permanently blocked.
///
/// The guard lifecycle is: false -> true (before callback) -> false (after
/// repayment verified). If the guard remains true after completion, all
/// mutating endpoints would be permanently locked.
#[rule]
fn flash_loan_guard_cleared_after_completion(
    e: Env,
    caller: Address,
    receiver: Address,
    asset: Address,
    amount: i128,
    data: Bytes,
) {
    cvlr_assume!(amount > 0);

    // Guard must be false before the flash loan
    cvlr_assume!(!crate::storage::is_flash_loan_ongoing(&e));

    // Execute the flash loan (will revert if repayment insufficient)
    crate::flash_loan::process_flash_loan(&e, &caller, &asset, amount, &receiver, &data);

    // After successful completion, the guard must be cleared
    cvlr_assert!(!crate::storage::is_flash_loan_ongoing(&e));
}

// ---------------------------------------------------------------------------
// Sanity
// ---------------------------------------------------------------------------

#[rule]
fn flash_loan_sanity(
    e: Env,
    caller: Address,
    receiver: Address,
    asset: Address,
    amount: i128,
    data: Bytes,
) {
    cvlr_assume!(amount > 0);
    crate::Controller::flash_loan(e, caller, asset, amount, receiver, data);
    cvlr_satisfy!(true);
}
