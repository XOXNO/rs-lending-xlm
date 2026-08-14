//! Ghost state for the post-pool solvency gate.
//!
//! `record_gate` is driven by `spec_hooks::solvency_gate_checked`, which
//! production calls from `risk::validation::require_post_pool_risk_gates`.
//! Besides the "the gate ran" flag the health rules already consume, it
//! snapshots the exact supply and debt books the gate valued, so a rule can
//! prove that snapshot is still the account's persisted state at end of
//! transaction — the fence against Trail of Bits' TOB-AAVE-7 ordering bug,
//! where Aave V4 validated health and *then* added debt.

use common::types::{Account, AccountPositionRaw, DebtPositionRaw, HubAssetKey};
use soroban_sdk::{Env, Map};

static mut GHOST_HF_CHECKED: bool = false;
static mut GHOST_GATE_OBSERVED: bool = false;
static mut GHOST_GATE_SUPPLY: Option<Map<HubAssetKey, AccountPositionRaw>> = None;
static mut GHOST_GATE_DEBT: Option<Map<HubAssetKey, DebtPositionRaw>> = None;

pub fn reset() {
    unsafe {
        GHOST_HF_CHECKED = false;
        GHOST_GATE_OBSERVED = false;
        GHOST_GATE_SUPPLY = None;
        GHOST_GATE_DEBT = None;
    }
}

/// Records one execution of the post-pool solvency gate together with the
/// position books it valued. A later gate in the same transaction overwrites
/// an earlier one: the admitting observation is the last one.
pub fn record_gate(account: &Account) {
    unsafe {
        GHOST_HF_CHECKED = true;
        GHOST_GATE_OBSERVED = true;
        GHOST_GATE_SUPPLY = Some(account.supply_positions.clone());
        GHOST_GATE_DEBT = Some(account.borrow_positions.clone());
    }
}

pub fn get_checked() -> bool {
    unsafe { GHOST_HF_CHECKED }
}

/// Whether the post-pool solvency gate ran at all. It is skipped for
/// debt-free accounts, and never reached by verbs that carry no post-pool
/// gate, so the post-gate rules are implications keyed on this flag.
pub fn gate_observed() -> bool {
    unsafe { GHOST_GATE_OBSERVED }
}

/// The supply book the gate valued, or an empty book if it never ran.
pub fn observed_supply(env: &Env) -> Map<HubAssetKey, AccountPositionRaw> {
    unsafe {
        match &*core::ptr::addr_of!(GHOST_GATE_SUPPLY) {
            Some(positions) => positions.clone(),
            None => Map::new(env),
        }
    }
}

/// The debt book the gate valued, or an empty book if it never ran.
pub fn observed_debt(env: &Env) -> Map<HubAssetKey, DebtPositionRaw> {
    unsafe {
        match &*core::ptr::addr_of!(GHOST_GATE_DEBT) {
            Some(positions) => positions.clone(),
            None => Map::new(env),
        }
    }
}
