/// Position Integrity Rules
///
/// Verify that add/remove operations maintain consistent position state.
/// Mirrors Blend's user_rules.rs pattern.
///
/// From CLAUDE.md:
///   - Max 10 positions per type (gas safety for liquidation iteration)
///   - Position type must remain consistent with storage key
///   - Sum(user_scaled) <= total_scaled — no phantom liquidity
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::types::{POSITION_TYPE_BORROW, POSITION_TYPE_DEPOSIT};

// ---------------------------------------------------------------------------
// Rule 1: Supply increases deposit position
// ---------------------------------------------------------------------------

/// After a successful supply, the user's deposit scaled amount for that asset
/// must increase (or be created if it didn't exist).
#[rule]
fn supply_increases_position(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    // Get position before (0 if doesn't exist)
    let pos_before =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_DEPOSIT, &asset);

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset.clone(), amount);

    let pos_after =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_DEPOSIT, &asset);

    cvlr_assert!(pos_after > pos_before);
}

// ---------------------------------------------------------------------------
// Rule 2: Borrow increases debt position
// ---------------------------------------------------------------------------

/// After a successful borrow, the user's borrow scaled amount must increase.
#[rule]
fn borrow_increases_debt(e: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);

    let pos_before =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_BORROW, &asset);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset.clone(), amount);

    let pos_after =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_BORROW, &asset);

    cvlr_assert!(pos_after > pos_before);
}

// ---------------------------------------------------------------------------
// Rule 3: Full repay clears debt position
// ---------------------------------------------------------------------------

/// After repaying the full debt, the borrow position must be zero (removed).
#[rule]
fn full_repay_clears_debt(e: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    // Amount must be large enough to cover full debt (use i128::MAX for "repay all")
    cvlr_assume!(amount == i128::MAX);

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset.clone(), amount);

    let pos_after =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_BORROW, &asset);

    cvlr_assert!(pos_after == 0);
}

// ---------------------------------------------------------------------------
// Rule 4: Withdraw decreases deposit position
// ---------------------------------------------------------------------------

#[rule]
fn withdraw_decreases_position(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let pos_before =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_DEPOSIT, &asset);
    cvlr_assume!(pos_before > 0);

    crate::spec::compat::withdraw_single(e.clone(), caller, account_id, asset.clone(), amount);

    let pos_after =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_DEPOSIT, &asset);

    cvlr_assert!(pos_after < pos_before);
}

// ---------------------------------------------------------------------------
// Rule 5: Repay decreases debt position
// ---------------------------------------------------------------------------

#[rule]
fn repay_decreases_debt(e: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);

    let pos_before =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_BORROW, &asset);
    cvlr_assume!(pos_before > 0);

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset.clone(), amount);

    let pos_after =
        crate::storage::positions::get_scaled_amount(&e, account_id, POSITION_TYPE_BORROW, &asset);

    cvlr_assert!(pos_after < pos_before);
}

// ---------------------------------------------------------------------------
// Sanity
// ---------------------------------------------------------------------------

#[rule]
fn supply_sanity(e: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);
    crate::spec::compat::supply_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(true);
}
