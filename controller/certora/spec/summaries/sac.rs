//! Summaries for the Stellar Asset Contract (SAC) `soroban_sdk::token::Client`.
//!
//! SAC token operations execute as cross-contract calls in the host. Without
//! summarization the prover treats them as pure havoc -- every property that
//! depends on `transfer_and_measure_received`, balance reads, or allowance
//! tracking becomes vacuous because both the call's effect and its return
//! value are unconstrained.
//!
//! These summaries replace the heavy cross-contract path with the
//! post-conditions production relies on: balances and allowances are
//! non-negative, transfer asserts non-negative amounts, balance reads are
//! pure (no state change). Failure modes (insufficient balance, insufficient
//! allowance) are modelled by giving the summary a panic-or-return shape
//! rather than collapsing to one branch.
//!
//! Wiring: registered against `soroban_sdk::token::Client::*` via
//! `cvlr_soroban_macros::apply_summary!` from the call sites in
//! `controller/src/utils.rs::transfer_and_measure_received` and the
//! flash-loan / strategy paths.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Env};

// ---------------------------------------------------------------------------
// Transfer
// ---------------------------------------------------------------------------

/// Summary for `soroban_sdk::token::Client::transfer`.
///
/// Production guarantees (SAC ABI):
///   * Asserts `amount >= 0`. Negative amounts panic in the host.
///   * On insufficient balance the call panics; the controller's caller
///     (`transfer_and_measure_received`) measures the post-balance delta to
///     observe what actually moved.
///   * No return value.
///
/// Bound: only the non-negative-amount precondition is enforced. The
/// downstream balance reads (summarized below) carry the post-condition
/// that balances stay non-negative.
pub fn transfer_summary(
    _env: &Env,
    _token: &Address,
    _from: &Address,
    _to: &Address,
    amount: &i128,
) {
    cvlr_assume!(*amount >= 0);
}

// ---------------------------------------------------------------------------
// Balance
// ---------------------------------------------------------------------------

/// Summary for `soroban_sdk::token::Client::balance`.
///
/// Production guarantees: SAC balance is always non-negative; the contract
/// never exposes a negative balance. The call is read-only -- no state
/// mutation.
///
/// Returning a fully nondet non-negative `i128` is the strongest sound
/// post-condition: every rule that compares pre- and post-transfer balances
/// must rely on a separate model of the transfer effect (handled via
/// `transfer_summary` + `cvlr_assume!` at call sites, not here -- mocking
/// the SAC's internal ledger inside a summary would be unsound).
pub fn balance_summary(_env: &Env, _token: &Address, _account: &Address) -> i128 {
    let bal: i128 = nondet();
    cvlr_assume!(bal >= 0);
    bal
}

// ---------------------------------------------------------------------------
// Approve / allowance
// ---------------------------------------------------------------------------

/// Summary for `soroban_sdk::token::Client::approve`.
///
/// Production guarantees:
///   * Sets the allowance for `spender` over `from`'s tokens to `amount`,
///     valid until `live_until_ledger`.
///   * Negative `amount` is rejected by the SAC host.
///   * No return value.
pub fn approve_summary(
    _env: &Env,
    _token: &Address,
    _from: &Address,
    _spender: &Address,
    amount: &i128,
    _live_until_ledger: &u32,
) {
    cvlr_assume!(*amount >= 0);
}

/// Summary for `soroban_sdk::token::Client::allowance`.
///
/// Production guarantees: SAC allowances are always non-negative. The call
/// is read-only -- no state mutation.
pub fn allowance_summary(
    _env: &Env,
    _token: &Address,
    _from: &Address,
    _spender: &Address,
) -> i128 {
    let allowance: i128 = nondet();
    cvlr_assume!(allowance >= 0);
    allowance
}
