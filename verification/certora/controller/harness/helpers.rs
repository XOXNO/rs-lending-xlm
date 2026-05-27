//! Certora harness substitute for `controller::helpers`.
//!
//! Under `--features certora`, `controller/src/lib.rs` path-swaps the
//! entire `helpers` module to this file (see lib.rs:9 and cross_contract
//! pattern for comparison). This is the most invasive override in the
//! controller harness layer because `helpers` acts as the shared
//! primitives bucket (dust checks, account lifecycle, position map
//! maintenance, and the heavy aggregate helpers) after a prior move
//! from positions/ for visibility to strategies and core verbs.
//!
//! The substitute:
//! - Pulls prod helpers/mod.rs via inner #[path] as __prod only for
//!   re-export of the non-heavy fns (explicit list prevents silent drift).
//! - Overrides [`calculate_account_totals`] with a sound nondet summary
//!   (heavy Map iteration + I256).
//! - Provides [`calculate_linear_bonus`] (certora-only, used by rules;
//!   mirrors the boundary of the prod `calculate_linear_bonus_with_target`).
//!
//! Known limitation (see oracle providers/*/client.rs success story):
//! full module replacement here couples harness maintenance to every
//! internal refactoring of helpers. Prefer thin-wrapper + apply_summary!
//! for future heavy fns. The calculate_linear_bonus + estimate fns that
//! rules expect are intentionally not forced into this surface.
//!
//! Migration target: Move `calculate_account_totals` to thin-wrapper + summarized! pattern.

#[allow(dead_code)] // Prod has fns we don't re-export (we shadow them locally).
#[path = "../../../../contracts/controller/src/helpers/mod.rs"]
mod __prod;

pub use __prod::{
    calculate_health_factor,
    calculate_ltv_collateral_wad,
    calculate_total_debt_wad,
    cleanup_account_if_empty,
    create_account,
    position_value,
    remove_account,
    require_no_borrow_dust_for_assets,
    require_no_supply_dust_for_assets,
    update_or_remove_debt_position,
    update_or_remove_supply_position,
    weighted_collateral,
    // NOTE: estimate_liquidation_amount / get_account_bonus_params live in
    // positions/liquidation_math (rules call via full path or via estimate
    // wrappers). calculate_account_totals and calculate_linear_bonus are
    // overridden locally below.
};

use common::math::fp::{Bps, Wad};
use common::types::{AccountPosition, AccountPositionRaw, DebtPositionRaw};
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
    _supply_positions: &Map<Address, AccountPositionRaw>,
    _borrow_positions: &Map<Address, DebtPositionRaw>,
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
