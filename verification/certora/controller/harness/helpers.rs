//! Certora harness substitute for `controller::helpers`.
//!
//! Under `--features certora`, `controller/src/lib.rs` path-swaps the
//! `helpers` module to this file. The substitute re-exports every
//! production helper unchanged except for the two paths that the
//! prover must see as summaries:
//!
//! 1. [`calculate_account_totals`] — heavy I256 + Map-iteration body
//!    replaced with a bounded nondet aggregate. Without the summary,
//!    every rule that traverses this helper hits the prover's TAC
//!    command-count budget.
//! 2. [`calculate_linear_bonus`] — certora-only thin wrapper around
//!    `calculate_linear_bonus_with_target` (target = 1.02 WAD). No
//!    production caller, so it lives here rather than polluting prod.
//!
//! The re-export list is enumerated explicitly so adding a new public
//! helper in production surfaces here as a compile error — the harness
//! cannot silently drift from the production surface.

#[allow(dead_code)] // Prod has fns we don't re-export (we shadow them locally).
#[path = "../../../../contracts/controller/src/helpers/mod.rs"]
mod __prod;

pub use __prod::{
    calculate_health_factor, calculate_ltv_collateral_wad, estimate_liquidation_amount,
    get_account_bonus_params, position_value, weighted_collateral,
};

use common::math::fp::{Bps, Wad};
use common::types::AccountPosition;
use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Env, Map};

use crate::cache::ControllerCache;

// ---------------------------------------------------------------------------
// Summary: `calculate_account_totals`
// ---------------------------------------------------------------------------
//
// Production iterates `supply_positions` + `borrow_positions` and
// aggregates per-asset USD-WAD values via the cache. The summary
// returns nondet aggregates under the production-side invariant
// `weighted_coll <= total_collateral` (per-asset weight ≤ value).
pub fn calculate_account_totals(
    _env: &Env,
    _cache: &mut ControllerCache,
    _supply_positions: &Map<Address, AccountPosition>,
    _borrow_positions: &Map<Address, AccountPosition>,
) -> (Wad, Wad, Wad) {
    let total_collateral_raw: i128 = nondet();
    let total_debt_raw: i128 = nondet();
    let weighted_coll_raw: i128 = nondet();
    cvlr_assume!(total_collateral_raw >= 0);
    cvlr_assume!(total_debt_raw >= 0);
    cvlr_assume!(weighted_coll_raw >= 0);
    cvlr_assume!(weighted_coll_raw <= total_collateral_raw);
    (
        Wad::from_raw(total_collateral_raw),
        Wad::from_raw(total_debt_raw),
        Wad::from_raw(weighted_coll_raw),
    )
}

// ---------------------------------------------------------------------------
// Certora-only helper: `calculate_linear_bonus`
// ---------------------------------------------------------------------------
//
// Spec rules in `liquidation_rules.rs` call this thin wrapper with the
// production target of 1.02 WAD. Summary pins the production boundary:
// when HF >= 1.02 WAD, production's `calculate_linear_bonus_with_target`
// early-returns `base` unchanged. Below the target the bonus is
// interpolated between `base` and `max`.
pub fn calculate_linear_bonus(_env: &Env, hf: Wad, base_bonus: Bps, max_bonus: Bps) -> Bps {
    let bonus_raw: i128 = nondet();
    cvlr_assume!(bonus_raw >= base_bonus.raw());
    cvlr_assume!(bonus_raw <= max_bonus.raw());
    let target_hf_wad: i128 = 102 * common::constants::WAD / 100;
    if hf.raw() >= target_hf_wad {
        cvlr_assume!(bonus_raw == base_bonus.raw());
    }
    Bps::from_raw(bonus_raw)
}
