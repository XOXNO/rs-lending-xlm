//! Function summaries for Certora verification.
//!
//! Each summary replaces a heavy production function with a sound, abstract
//! over-approximation. Under the `certora` feature, calls to the original
//! function are redirected here via `cvlr_soroban_macros::apply_summary!`,
//! which wraps the function definition in place at its source site. The real
//! body still compiles when the feature is off.
//!
//! Why summarize:
//!   * Heavy I256 / bytemap / map-iteration paths blow up the prover's TAC
//!     command count (the `1786191 > 1000000` errors we saw).
//!   * Cross-contract `LiquidityPoolClient` calls are pure havoc to the
//!     prover; explicit nondet returns are equivalent semantically and
//!     orders of magnitude cheaper.
//!   * Math primitives like `mul_div_half_up` already have dedicated rules
//!     in `math_rules`; no other rule should re-prove them by inlining.
//!
//! Soundness contract: every summary returns a value in the same domain as
//! the production function and assumes only properties production GUARANTEES
//! (post-conditions). If a summary assumes more, it weakens correctness.
//!
//! Verifying the summary itself: dedicated rules in `oracle_rules`,
//! `health_rules`, etc. exercise the real production function (via
//! `crate::oracle::token_price::token_price` -- the unsummarised
//! sub-module that `apply_summary!` preserves). If a summary's pre/post
//! contract drifts from production, those rules fail.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Env};

use common::fp::{Bps, Wad};
use common::types::{MarketIndex, PriceFeed};

use crate::cache::ControllerCache;

// Cross-contract summaries split into their own modules to keep the file
// boundary aligned with the contract being summarised.
//   * `pool`  -- the `LiquidityPool` ABI in `pool/src/lib.rs`.
//   * `sac`   -- the SAC `soroban_sdk::token::Client` ABI.
pub mod pool;
pub mod sac;

// ---------------------------------------------------------------------------
// Oracle summaries
// ---------------------------------------------------------------------------

/// Summary for `crate::oracle::token_price`.
///
/// Production guarantees (post-conditions):
///   * `price_wad > 0` (zero-or-negative panics with `InvalidPrice`).
///   * `asset_decimals` discovered on-chain at config time; bounded `<= 27`
///     by the `RAY_DECIMALS` ceiling.
///   * `timestamp <= cache.current_timestamp_ms / 1000 + 60` (the staleness
///     guard rejects feeds further in the future than the 60-s clock-skew
///     tolerance).
pub fn token_price_summary(cache: &mut ControllerCache, _asset: &Address) -> PriceFeed {
    let price_wad: i128 = nondet();
    let asset_decimals: u32 = nondet();
    let timestamp: u64 = nondet();
    cvlr_assume!(price_wad > 0);
    cvlr_assume!(asset_decimals <= 27);
    cvlr_assume!(timestamp <= cache.current_timestamp_ms / 1000 + 60);
    PriceFeed {
        price_wad,
        asset_decimals,
        timestamp,
    }
}

/// Summary for `crate::oracle::is_within_anchor`.
///
/// Production guarantee: returns a boolean. The real implementation does an
/// I256 ratio computation and BPS rescale; for rules that only care WHICH
/// branch fires, a nondet bool is sound.
pub fn is_within_anchor_summary(
    _env: &Env,
    _aggregator: i128,
    _safe: i128,
    _upper_bound_ratio: i128,
    _lower_bound_ratio: i128,
) -> bool {
    nondet()
}

/// Summary for `crate::oracle::update_asset_index`.
///
/// Production reads the pool's current sync data (cross-contract) and
/// simulates interest accrual. The summary returns a fresh `MarketIndex`
/// satisfying the index-monotonicity post-conditions:
///   * `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW` (= `WAD`, the bad-debt
///     floor).
///   * `borrow_index_ray >= RAY` (initial value; only grows).
///   * `last_timestamp <= cache.current_timestamp_ms`.
pub fn update_asset_index_summary(
    _cache: &mut ControllerCache,
    _asset: &Address,
) -> MarketIndex {
    let supply_index_ray: i128 = nondet();
    let borrow_index_ray: i128 = nondet();
    cvlr_assume!(supply_index_ray >= common::constants::SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(borrow_index_ray >= common::constants::RAY);
    // No `borrow_index_ray >= supply_index_ray` bound: production allows the
    // supply index to drop below the borrow index after `pool::seize_position`
    // calls `apply_bad_debt_to_supply_index` (pool/src/lib.rs:521-525).
    MarketIndex {
        supply_index_ray,
        borrow_index_ray,
    }
}

// ---------------------------------------------------------------------------
// Health-factor and account-totals summaries
// ---------------------------------------------------------------------------

/// Summary for `crate::helpers::calculate_health_factor`.
///
/// Production iterates supply / borrow position maps, computes weighted USD
/// values, and divides via I256 with `i128::MAX` saturation
/// (`controller/src/helpers/mod.rs:100-113`).
///
/// Why the bound is tied to the input maps (Option A): a plain
/// `cvlr_assume!(hf >= 0)` returns an independent fresh draw on every call,
/// so any rule asserting "after operation X, HF satisfies P" would be checking
/// a fresh nondet value, not the value the production function actually
/// produced. A buggy `calculate_health_factor` returning, say, `WAD - 1` for
/// every undercollateralized account would still pass every rule. Tying the
/// summary's domain to observable input shape (empty borrow → MAX, empty
/// supply with debt → 0) is the minimum constraint that makes post-state HF
/// rules meaningful.
pub fn calculate_health_factor_summary(
    _env: &Env,
    _cache: &mut ControllerCache,
    supply_positions: &soroban_sdk::Map<Address, common::types::AccountPosition>,
    borrow_positions: &soroban_sdk::Map<Address, common::types::AccountPosition>,
) -> i128 {
    let hf: i128 = nondet();
    cvlr_assume!(hf >= 0);
    // No-debt accounts saturate to `i128::MAX` (helpers/mod.rs:100-101).
    if borrow_positions.is_empty() {
        cvlr_assume!(hf == i128::MAX);
    } else if supply_positions.is_empty() {
        // Non-empty borrow with empty supply: numerator is zero, division
        // yields zero.
        cvlr_assume!(hf == 0);
    }
    hf
}

#[cfg(feature = "certora")]
/// Summary for `crate::helpers::calculate_health_factor_for`.
///
/// Looks up the account from storage and dispatches to the same input-tied
/// bounds as `calculate_health_factor_summary`. See the doc comment on that
/// summary for why the nondet draw is constrained by the account's position
/// maps rather than left fully unconstrained.
pub fn calculate_health_factor_for_summary(
    env: &Env,
    _cache: &mut ControllerCache,
    account_id: u64,
) -> i128 {
    let hf: i128 = nondet();
    cvlr_assume!(hf >= 0);
    if let Some(account) = crate::storage::try_get_account(env, account_id) {
        if account.borrow_positions.is_empty() {
            cvlr_assume!(hf == i128::MAX);
        } else if account.supply_positions.is_empty() {
            cvlr_assume!(hf == 0);
        }
    }
    hf
}

/// Summary for `crate::helpers::calculate_account_totals`.
///
/// Production returns `(total_collateral, total_debt, weighted_coll)` (see
/// `controller/src/helpers/mod.rs:184`). The summary mirrors that order
/// exactly. The weighted collateral is bounded by the total collateral
/// (production invariant `weighted_coll = Σ value × LT_bps / BPS <= Σ value`).
pub fn calculate_account_totals_summary(
    _env: &Env,
    _cache: &mut ControllerCache,
    _supply_positions: &soroban_sdk::Map<Address, common::types::AccountPosition>,
    _borrow_positions: &soroban_sdk::Map<Address, common::types::AccountPosition>,
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
// Bonus summary -- the linear interpolation at `helpers::calculate_linear_bonus`
// ---------------------------------------------------------------------------

/// Summary for `crate::helpers::calculate_linear_bonus`.
///
/// Production linearly interpolates between `base_bonus` and `max_bonus`
/// based on how far HF sits below `1.02 WAD`. The summary returns a `Bps`
/// value in `[base_bonus, max_bonus]` -- the only bound any rule asserts.
pub fn calculate_linear_bonus_summary(
    _env: &Env,
    _hf: Wad,
    base_bonus: Bps,
    max_bonus: Bps,
) -> Bps {
    let bonus_raw: i128 = nondet();
    cvlr_assume!(bonus_raw >= base_bonus.raw());
    cvlr_assume!(bonus_raw <= max_bonus.raw());
    Bps::from_raw(bonus_raw)
}

// ---------------------------------------------------------------------------
// Account-USD-aggregate view summaries
// ---------------------------------------------------------------------------

/// Summary for `crate::views::total_collateral_in_usd`.
///
/// Production iterates supply_assets and sums per-asset USD-WAD values via
/// the cache. Returns a non-negative i128 (zero-account branch returns 0;
/// non-empty branches return a non-negative aggregate Wad raw value).
pub fn total_collateral_in_usd_summary(_env: &Env, _account_id: u64) -> i128 {
    let total: i128 = nondet();
    cvlr_assume!(total >= 0);
    total
}

/// Summary for `crate::views::total_borrow_in_usd`. Same shape as above.
pub fn total_borrow_in_usd_summary(_env: &Env, _account_id: u64) -> i128 {
    let total: i128 = nondet();
    cvlr_assume!(total >= 0);
    total
}

/// Summary for `crate::views::ltv_collateral_in_usd`.
///
/// Production caps the per-asset weight at `loan_to_value_bps`, so the
/// result is bounded by `total_collateral_in_usd`. The summary returns a
/// non-negative i128 -- the per-rule LTV assertion checks the relationship
/// against `total_borrow` separately.
pub fn ltv_collateral_in_usd_summary(_env: &Env, _account_id: u64) -> i128 {
    let total: i128 = nondet();
    cvlr_assume!(total >= 0);
    total
}
