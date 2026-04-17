/// Health Factor Invariant Rules
///
/// From CLAUDE.md:
///   - HF uses current (synced) indexes, never stale values
///   - HF >= 1.0 required after every borrow/withdraw operation
///   - HF < 1.0 required to trigger liquidation (no wrongful liquidations)
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::WAD;
// ---------------------------------------------------------------------------
// Rule 1: Health factor >= 1.0 WAD after borrow
// ---------------------------------------------------------------------------

/// After any successful borrow, the borrower's health factor must be >= 1.0.
/// This prevents the protocol from issuing undercollateralized loans.
#[rule]
fn hf_safe_after_borrow(e: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    // Assume valid inputs
    cvlr_assume!(amount > 0);

    // Execute borrow (will panic if HF check fails internally)
    crate::spec::compat::borrow_single(e.clone(), caller.clone(), account_id, asset, amount);

    // Verify: HF must be >= 1.0 WAD after successful borrow
    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let hf = crate::helpers::calculate_health_factor_for(&e, &mut cache, account_id);
    cvlr_assert!(hf >= WAD);
}

// ---------------------------------------------------------------------------
// Rule 2: Health factor >= 1.0 WAD after withdraw
// ---------------------------------------------------------------------------

/// After any successful withdraw, the withdrawer's health factor must be >= 1.0
/// (if they have outstanding borrows).
#[rule]
fn hf_safe_after_withdraw(e: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);

    crate::spec::compat::withdraw_single(e.clone(), caller.clone(), account_id, asset, amount);

    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let hf = crate::helpers::calculate_health_factor_for(&e, &mut cache, account_id);

    // If account has borrows, HF must be >= 1.0
    // If no borrows, HF is i128::MAX (infinite), which is >= WAD
    cvlr_assert!(hf >= WAD);
}

// ---------------------------------------------------------------------------
// Rule 3: Liquidation requires HF < 1.0 (no wrongful liquidations)
// ---------------------------------------------------------------------------

/// Liquidation must only be possible when the account is unhealthy (HF < 1.0).
/// The rule constrains HF to be healthy, calls liquidation, and treats any
/// reachable success path as a violation.
#[rule]
fn liquidation_requires_unhealthy_account(
    e: Env,
    liquidator: Address,
    account_id: u64,
    debt_asset: Address,
    debt_amount: i128,
) {
    cvlr_assume!(debt_amount > 0);

    // Assume the account is healthy (HF >= 1.0 WAD)
    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let hf_before = crate::helpers::calculate_health_factor_for(&e, &mut cache, account_id);
    cvlr_assume!(hf_before >= WAD);

    // Build debt payments and attempt liquidation
    let mut payments: soroban_sdk::Vec<(Address, i128)> = soroban_sdk::Vec::new(&e);
    payments.push_back((debt_asset, debt_amount));

    // This call must revert because the account is healthy.
    crate::positions::liquidation::process_liquidation(&e, &liquidator, account_id, &payments);

    // If execution reaches here, liquidation succeeded on a healthy account.
    // The prover must show this line is unreachable -- otherwise the rule fails.
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 4: Supply only improves or maintains health factor
// ---------------------------------------------------------------------------

/// Supplying collateral must not decrease an account's health factor.
/// (It should stay the same or improve.)
#[rule]
fn supply_cannot_decrease_hf(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let hf_before = crate::helpers::calculate_health_factor_for(&e, &mut cache, account_id);

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset, amount);

    let mut cache2 = crate::cache::ControllerCache::new(&e, false);
    let hf_after = crate::helpers::calculate_health_factor_for(&e, &mut cache2, account_id);

    cvlr_assert!(hf_after >= hf_before);
}

// ---------------------------------------------------------------------------
// Sanity rules (reachability checks -- ensures rules aren't vacuously true)
// ---------------------------------------------------------------------------

#[rule]
fn hf_borrow_sanity(e: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);
    crate::spec::compat::borrow_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(true);
}

#[rule]
fn hf_withdraw_sanity(e: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);
    crate::spec::compat::withdraw_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(true);
}
