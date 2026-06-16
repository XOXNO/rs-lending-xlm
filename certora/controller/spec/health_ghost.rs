//! Ghost state for the Blend-style "health-gated" proof.
//!
//! Mirrors the Certora/Blend `pool/src/spec/model.rs` `GHOST_CHECKED` pattern:
//! a flag set by the production solvency gate (`require_post_pool_risk_gates`)
//! once its collateral-covers-debt assertion has run. A rule can then assert
//! "for an arbitrary reserve, either the user moved in a safe direction OR the
//! solvency gate executed", which proves every risk-increasing operation is
//! gated without re-deriving the (summarised) aggregate health math.
//!
//! The production call site is `#[cfg(feature = "certora")]`, so this state
//! exists only in the prover build.

static mut GHOST_HF_CHECKED: bool = false;

/// Reset before the operation under test. Each rule calls this first so the
/// flag reflects only the operation it exercises.
pub fn reset() {
    unsafe { GHOST_HF_CHECKED = false }
}

/// Set by `require_post_pool_risk_gates` after the collateral-covers-debt
/// assertion runs.
pub fn set_checked() {
    unsafe { GHOST_HF_CHECKED = true }
}

pub fn get_checked() -> bool {
    unsafe { GHOST_HF_CHECKED }
}
