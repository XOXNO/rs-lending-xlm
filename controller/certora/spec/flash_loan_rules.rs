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
// Rule 3: Flash-loan guard helper rejects calls when the flag is set
// ---------------------------------------------------------------------------

/// Every mutating controller endpoint calls `require_not_flash_loaning`
/// before any other work; if that helper rejects when the flag is set, all
/// downstream paths are blocked.
///
/// We verify the helper directly. The previous formulation called the full
/// `borrow_single` flow, which expanded to ~129k basic blocks (over the
/// `maxBlockCount` ceiling). The narrow form below is < 5k blocks and
/// generalises: any future mutating endpoint that calls
/// `require_not_flash_loaning` first inherits the property by construction.
#[rule]
fn flash_loan_guard_blocks_callers(e: Env) {
    crate::storage::set_flash_loan_ongoing(&e, true);

    // Production guard: panics with `FlashLoanError::FlashLoanOngoing`.
    crate::validation::require_not_flash_loaning(&e);

    // Unreachable: if the guard helper does not panic when the flag is
    // set, the rule fails -- which means a mutating endpoint could leak
    // through the guard at the call site too.
    cvlr_satisfy!(false);
}

/// Companion: when the flag is NOT set, the guard helper returns cleanly.
/// Catches a regression where the guard panics unconditionally (which would
/// permanently lock the protocol).
#[rule]
fn flash_loan_guard_allows_when_clear(e: Env) {
    crate::storage::set_flash_loan_ongoing(&e, false);

    crate::validation::require_not_flash_loaning(&e);

    // Reachable: the guard returns and we get here.
    cvlr_satisfy!(true);
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
