//! SAC token summaries: non-negative amounts, balances, and allowances.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Env};

/// Transfer: requires `amount >= 0`.
pub fn transfer_summary(
    _env: &Env,
    _token: &Address,
    _from: &Address,
    _to: &Address,
    amount: &i128,
) {
    cvlr_assume!(*amount >= 0);
}

/// Balance read: non-negative, no state change.
pub fn balance_summary(_env: &Env, _token: &Address, _account: &Address) -> i128 {
    let bal: i128 = nondet();
    cvlr_assume!(bal >= 0);
    bal
}

/// Approve: requires `amount >= 0`.
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

/// Allowance read: non-negative, no state change.
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