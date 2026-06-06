//! Function summaries for Certora verification.
//!
//! Each summary replaces a heavy production function with a sound, abstract
//! over-approximation. Under the `certora` feature, calls to the original
//! function are redirected here via `cvlr_soroban_macros::apply_summary!`,
//! which wraps the function definition in place at its source site. The real
//! body still compiles when the feature is off.
//!
//! Summary rationale:
//!   * Heavy I256, bytemap, and map-iteration paths can exceed prover command
//!     limits.
//!   * Cross-contract `LiquidityPoolClient` calls are pure havoc to the
//!     prover; explicit nondet returns provide equivalent abstraction with
//!     lower verification cost.
//!   * Math primitives like `mul_div_half_up` already have dedicated rules
//!     in `math_rules`; other rules avoid re-proving them by inlining.
//!
//! Soundness contract: every summary must return a value in the same domain as
//! the production function or explicitly document any narrowed branch it
//! models. Over-constraining a summary weakens verification.
//!
//! Verifying the summary itself: dedicated rules in `oracle_rules`,
//! `health_rules`, etc. exercise the real production function (via
//! `crate::oracle::token_price::token_price` -- the unsummarised
//! sub-module that `apply_summary!` preserves). If a summary's pre/post
//! contract drifts from production, those rules provide the targeted check.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Env};

use common::math::fp::{Bps, Wad};
use common::types::{MarketIndex, PriceFeedRaw};

use crate::cache::Cache;

// Cross-contract summaries split into their own modules to keep the file
// boundary aligned with the contract being summarised.
//   * `pool`       -- the `LiquidityPool` ABI in `pool/src/lib.rs`.
//   * `sac`        -- the SAC `soroban_sdk::token::Client` ABI.
//   * `reflector`  -- the SEP-40 Reflector oracle ABI in
//     `controller/src/oracle/providers/reflector/client.rs` (canonical home).
pub mod pool;
pub mod reflector;
pub mod sac;
// Oracle summaries

/// Summary for `crate::oracle::token_price`.
///
/// This summary replaces the entire curated price resolution logic
/// (providers + compose + tolerance + validation) so the prover does not
/// traverse the real implementation in `oracle/`.
///
/// Production guarantees (post-conditions):
///   * `price_wad > 0` (zero-or-negative panics with `InvalidPrice`).
///   * `asset_decimals` discovered on-chain at config time; bounded `<= 27`
///     by the `RAY_DECIMALS` ceiling.
///   * `timestamp <= cache.current_timestamp_ms / 1000 + 60` (the staleness
///     guard rejects feeds further in the future than the 60-s clock-skew
///     tolerance).
pub fn token_price_summary(cache: &mut Cache, _asset: &Address) -> PriceFeedRaw {
    let price_wad: i128 = nondet();
    let asset_decimals: u32 = nondet();
    let timestamp: u64 = nondet();
    cvlr_assume!(price_wad > 0);
    cvlr_assume!(asset_decimals <= 27);
    cvlr_assume!(timestamp <= cache.current_timestamp_ms / 1000 + 60);
    PriceFeedRaw {
        price_wad,
        asset_decimals,
        timestamp,
    }
}

/// Summary for `crate::oracle::is_within_anchor`.
///
/// The production implementation lives in the clean `oracle/tolerance.rs`.
/// It performs expensive fixed-point math; this summary returns a sound
/// nondet bool so rules can focus on the high-level tolerance policy
/// branches without paying the math cost.
pub fn is_within_anchor_summary(
    _env: &Env,
    _aggregator: i128,
    _safe: i128,
    _upper_bound_ratio: u32,
    _lower_bound_ratio: u32,
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
pub fn update_asset_index_summary(_cache: &mut Cache, _asset: &Address) -> MarketIndex {
    use common::math::fp::Ray;
    let supply_index_ray: i128 = nondet();
    let borrow_index_ray: i128 = nondet();
    cvlr_assume!(supply_index_ray >= common::constants::SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(borrow_index_ray >= common::constants::RAY);
    // No `borrow_index_ray >= supply_index_ray` bound: production allows the
    // supply index to drop below the borrow index after `pool::seize_position`
    // calls `apply_bad_debt_to_supply_index` (pool/src/lib.rs:521-525).
    MarketIndex {
        supply_index: Ray::from(supply_index_ray),
        borrow_index: Ray::from(borrow_index_ray),
    }
}
// Account-totals summary

/// Summary for `crate::helpers::calculate_account_totals`.
///
/// Production returns `(total_collateral, total_debt, weighted_coll)` (see
/// `controller/src/helpers/mod.rs:184`). The summary mirrors that order
/// exactly. The weighted collateral is bounded by the total collateral
/// (production invariant `weighted_coll = Σ value × LT_bps / BPS <= Σ value`).
pub fn calculate_account_totals_summary(
    _env: &Env,
    _cache: &mut Cache,
    _supply_positions: &soroban_sdk::Map<Address, common::types::AccountPositionRaw>,
    _borrow_positions: &soroban_sdk::Map<Address, common::types::DebtPositionRaw>,
) -> (Wad, Wad, Wad) {
    let total_collateral_raw: i128 = nondet();
    let total_debt_raw: i128 = nondet();
    let weighted_coll_raw: i128 = nondet();
    cvlr_assume!(total_collateral_raw >= 0);
    cvlr_assume!(total_debt_raw >= 0);
    cvlr_assume!(weighted_coll_raw >= 0);
    cvlr_assume!(weighted_coll_raw <= total_collateral_raw);
    (
        Wad::from(total_collateral_raw),
        Wad::from(total_debt_raw),
        Wad::from(weighted_coll_raw),
    )
}
// Bonus summary -- the linear interpolation at `helpers::calculate_linear_bonus`

/// Summary for `crate::helpers::calculate_linear_bonus`.
///
/// Production linearly interpolates between `base_bonus` and `max_bonus`
/// based on how far HF sits below `1.02 WAD`. When `HF >= target_hf`
/// (= 1.02 WAD) production returns *exactly* `base_bonus`
/// (`controller/src/helpers/mod.rs::calculate_linear_bonus_with_target` returns
/// `base` on the early-return path when `target - hf <= 0`). The summary
/// pins that boundary case so rules asserting `bonus == base_bonus` at
/// `HF >= 1.02 WAD` are not refuted by an unconstrained
/// `[base_bonus, max_bonus]` draw.
pub fn calculate_linear_bonus_summary(_env: &Env, hf: Wad, base_bonus: Bps, max_bonus: Bps) -> Bps {
    let bonus_raw: i128 = nondet();
    cvlr_assume!(bonus_raw >= base_bonus.raw());
    cvlr_assume!(bonus_raw <= max_bonus.raw());
    // Production target: 1.02 WAD. When `hf >= target`, the linear
    // interpolation early-returns `base` unchanged.
    let target_hf_wad: i128 = 102 * common::constants::WAD / 100;
    if hf.raw() >= target_hf_wad {
        cvlr_assume!(bonus_raw == base_bonus.raw());
    }
    Bps::from(bonus_raw)
}
// Account-USD-aggregate view summaries

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
