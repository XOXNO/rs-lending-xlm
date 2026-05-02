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
// Rule 2: Flash loan repayment covers borrowed + fee -- DELETED
// ---------------------------------------------------------------------------
//
// The earlier `flash_loan_fee_collected` rule compared two independent calls
// to `pool_client.protocol_revenue()` taken before and after the flash-loan
// invocation. With no `protocol_revenue` summary wired and no shared per-tx
// snapshot, both reads return independent havoced i128 values -- so the
// `revenue_after >= revenue_before` assertion was vacuous.
//
// The protocol-revenue monotonicity property belongs in the pool crate's
// spec (where the same storage backs both reads), or it requires a joint
// pool-views summary that draws both reads from a single snapshot. Both are
// out of scope for this controller-side spec module. See
// audit/certora-efficiency/06-strategy-flashloan.md, "flash_loan_fee_collected
// is mis-located" for rationale.

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
///
/// The guard lifecycle is: false -> true (before callback) -> false (after
/// repayment verified). If the guard remains true after completion, all
/// mutating endpoints would be permanently locked.
///
/// Audit P1a / `06-strategy-flashloan.md` flagged this rule as vacuously
/// satisfied on every revert path inside `process_flash_loan`: when any
/// pre-check (`require_amount_positive`, `require_market_active`,
/// `is_flashloanable`), the receiver callback, or `flash_loan_end` panics,
/// Soroban rolls the entire transaction back -- the guard is implicitly
/// cleared by rollback, not by the production code, so the assertion was
/// trivially satisfied via the unreachable post-state.
///
/// Below we exclude the controller-side revert paths via `cvlr_assume!`s so
/// the rule actually exercises the success path. The third-party callback
/// path (`env.invoke_contract::<()>` at flash_loan.rs:56-60) and the pool's
/// own `flash_loan_end` repay-shortfall panic are out of the controller's
/// reach; both remain vacuously satisfied via rollback. Pair with the
/// `flash_loan_guard_cleared_sanity` companion below to confirm the success
/// path is actually reachable under the wired summaries.
#[rule]
fn flash_loan_guard_cleared_after_completion(
    e: Env,
    caller: Address,
    receiver: Address,
    asset: Address,
    amount: i128,
    data: Bytes,
) {
    // Bound the controller-side revert paths so the rule's PASS does not
    // silently come from one of these short-circuits.
    cvlr_assume!(amount > 0); // require_amount_positive (validation.rs:45-49)
    cvlr_assume!(!crate::storage::is_flash_loan_ongoing(&e)); // require_not_flash_loaning

    // Constrain the asset to a flashloanable, active market. Otherwise
    // `is_flashloanable` (flash_loan.rs:42-44) and `require_market_active`
    // (flash_loan.rs:37) panic before the guard is ever set.
    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let cfg = cache.cached_asset_config(&asset);
    cvlr_assume!(cfg.is_flashloanable);
    let market = crate::storage::get_market_config(&e, &asset);
    cvlr_assume!(market.status == common::types::MarketStatus::Active);
    drop(cache); // production rebuilds its own cache inside process_flash_loan

    // Execute the flash loan. Third-party paths (callback panic, pool-side
    // repay-shortfall panic) are out of the controller's reach; on those
    // revert paths Soroban rolls back state, which is the expected behaviour.
    crate::flash_loan::process_flash_loan(&e, &caller, &asset, amount, &receiver, &data);

    // Successful path: production must clear the guard at flash_loan.rs:64.
    cvlr_assert!(!crate::storage::is_flash_loan_ongoing(&e));
}

/// Reachability check for the success path of `flash_loan_guard_cleared_after_completion`.
/// Without summaries that constrain the cross-contract callback's return, the
/// success path may not be feasible to the prover -- in which case the parent
/// rule passes vacuously. This sanity rule fails (does not satisfy) when the
/// success path is unreachable, surfacing the wiring gap.
#[rule]
fn flash_loan_guard_cleared_sanity(
    e: Env,
    caller: Address,
    receiver: Address,
    asset: Address,
    amount: i128,
    data: Bytes,
) {
    cvlr_assume!(amount > 0);
    cvlr_assume!(!crate::storage::is_flash_loan_ongoing(&e));

    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let cfg = cache.cached_asset_config(&asset);
    cvlr_assume!(cfg.is_flashloanable);
    let market = crate::storage::get_market_config(&e, &asset);
    cvlr_assume!(market.status == common::types::MarketStatus::Active);
    drop(cache);

    crate::flash_loan::process_flash_loan(&e, &caller, &asset, amount, &receiver, &data);

    // Reachability: if this never satisfies, the prover never finds a witness
    // for the post-state and the parent rule's PASS is vacuous.
    cvlr_satisfy!(!crate::storage::is_flash_loan_ongoing(&e));
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
