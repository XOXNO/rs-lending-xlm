/// Boundary Condition & Overflow Safety -- Formal Verification Rules
///
/// Certora Sunbeam rules that probe exact boundary values where behavior
/// changes. Each rule targets the precise edge (==, == -1, == +1) of a
/// protocol decision boundary to ensure no off-by-one errors exist.
///
/// Categories:
///   - Interest rate boundaries (0%, mid, optimal, 100%, max compound)
///   - Liquidation boundaries (HF == 1.0, HF just below, bonus at 1.02, bad debt)
///   - Precision boundaries (max i128 safe mul, Taylor accuracy, rescale)
///   - Oracle tolerance boundaries (exact first/second tier, just beyond)
///   - Position boundaries (dust supply, exact reserves borrow, over-withdraw)
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_satisfy};
use soroban_sdk::Env;

use common::constants::{MILLISECONDS_PER_YEAR, RAY, WAD};
use common::fp::Ray;
use common::fp_core::{div_by_int_half_up, mul_div_half_up, rescale_half_up};
use common::rates::{calculate_borrow_rate, compound_interest};
use common::types::MarketParams;

// ---------------------------------------------------------------------------
// Helper: build deterministic MarketParams for boundary tests
// ---------------------------------------------------------------------------

/// Builds a well-known set of market params with predictable boundary points.
/// base = 1%, slope1 = 4%, slope2 = 10%, slope3 = 80%, mid = 50%,
/// optimal = 80%, max = 100%.
///
/// The `asset_id` is intentionally `env.current_contract_address()` rather
/// than a parsed Address::from_str: the parsed-string path forces the prover
/// to keep a symbolic Address constant alive for every rule, which inflates
/// the path count. The test contract address is already a host primitive.
fn boundary_test_params(env: &Env) -> MarketParams {
    MarketParams {
        base_borrow_rate_ray: RAY / 100,         // 1%
        slope1_ray: RAY * 4 / 100,               // 4%
        slope2_ray: RAY * 10 / 100,              // 10%
        slope3_ray: RAY * 80 / 100,              // 80%
        mid_utilization_ray: RAY * 50 / 100,     // 50%
        optimal_utilization_ray: RAY * 80 / 100, // 80%
        max_borrow_rate_ray: RAY,                // 100%
        reserve_factor_bps: 1000,                // 10%
        asset_id: env.current_contract_address(),
        asset_decimals: 7,
    }
}

// ===========================================================================
// Interest Rate Boundary Tests
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 1: borrow_rate_at_exact_zero -- DELETED (strict-stronger duplicate).
// Coverage preserved by `interest_rules::borrow_rate_zero_utilization` over
// fully nondet `nondet_valid_params(e)`, which strictly dominates the fixed
// `boundary_test_params` case.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn borrow_rate_at_exact_zero_sanity(e: Env) {
    let params = boundary_test_params(&e);
    let rate = calculate_borrow_rate(&e, Ray::ZERO, &params);
    cvlr_satisfy!(rate.raw() > 0);
}

// ---------------------------------------------------------------------------
// Rule 2: borrow_rate_at_exact_mid -- DELETED (strict-stronger duplicate).
// Coverage preserved by `interest_rules::borrow_rate_continuity_at_mid`, which
// pins both `mid - 1` and `mid` and bounds the gap by 1 over nondet params.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn borrow_rate_at_exact_mid_sanity(e: Env) {
    let params = boundary_test_params(&e);
    let rate = calculate_borrow_rate(&e, Ray::from_raw(params.mid_utilization_ray), &params);
    cvlr_satisfy!(rate.raw() > 0);
}

// ---------------------------------------------------------------------------
// Rule 3: borrow_rate_at_exact_optimal -- DELETED (strict-stronger duplicate).
// Coverage preserved by `interest_rules::borrow_rate_continuity_at_optimal`,
// which pins both `optimal - 1` and `optimal` and bounds the gap by 1 over
// nondet params.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn borrow_rate_at_exact_optimal_sanity(e: Env) {
    let params = boundary_test_params(&e);
    let rate = calculate_borrow_rate(&e, Ray::from_raw(params.optimal_utilization_ray), &params);
    cvlr_satisfy!(rate.raw() > 0);
}

// ---------------------------------------------------------------------------
// Rule 4: borrow_rate_at_100_percent -- DELETED (strict-stronger duplicate).
// Coverage preserved by `interest_rules::borrow_rate_capped`, which asserts
// `rate <= cap + 1` for any utilization in `[0, RAY]` over nondet params.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn borrow_rate_at_100_percent_sanity(e: Env) {
    let params = boundary_test_params(&e);
    let rate = calculate_borrow_rate(&e, Ray::ONE, &params);
    cvlr_satisfy!(rate.raw() > 0);
}

// ---------------------------------------------------------------------------
// Rule 5: compound_interest_at_max_rate_max_time -- DELETED (subsumed).
// Coverage preserved by `interest_rules::compound_interest_monotonic_in_time`
// + `compound_interest_ge_simple` over nondet rate/time, which together imply
// the same bounded-growth property.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn compound_interest_at_max_rate_max_time_sanity(e: Env) {
    let rate_per_ms = div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&e, Ray::from_raw(rate_per_ms), MILLISECONDS_PER_YEAR);
    // e^1 ~= 2.718, so factor should be around 2.718 * RAY
    cvlr_satisfy!(factor.raw() > 2 * RAY && factor.raw() < 3 * RAY);
}

// ===========================================================================
// Liquidation Boundary Tests
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 6: liquidation_at_hf_exactly_one -- DELETED (vacuously refutable).
// Under the input-tied `calculate_health_factor_for_summary`, the
// `cvlr_assume!(hf == WAD)` collapses the summary to its unconstrained branch
// and `cvlr_assert!(hf >= WAD)` becomes a tautology over the assumption.
// The rule does not exercise the production guard; rewriting requires invoking
// `process_liquidation` and observing panic vs success, deferred until the
// HF summary is tightened with a per-account ghost coupling.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn liquidation_at_hf_exactly_one_sanity() {
    let hf = WAD;
    cvlr_satisfy!(hf >= WAD);
}

// ---------------------------------------------------------------------------
// Rule 7: liquidation_at_hf_just_below_one -- DELETED (vacuously refutable).
// Same shape as Rule 6: `cvlr_assume!(hf == WAD - 1)` then
// `cvlr_assert!(hf < WAD)` is a tautology over the assumption under the
// input-tied HF summary. Rewriting requires invoking `process_liquidation`
// and observing the outcome; deferred until the HF summary is tightened.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn liquidation_at_hf_just_below_one_sanity() {
    let hf = WAD - 1;
    cvlr_satisfy!(hf < WAD);
}

// ---------------------------------------------------------------------------
// Rule 8: bonus_at_hf_exactly_102 -- DELETED (fails the prover as written).
// `calculate_linear_bonus_summary` (summaries/mod.rs:209-219) admits any
// value in `[base, max]`, so a counterexample `bonus = max` is allowed and
// `cvlr_assert!(|bonus - base| <= 1)` fails. Reinstate after F2 lands (i.e.
// summary tightened to return exactly `base` when `hf >= target_hf`).
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn bonus_at_hf_exactly_102_sanity() {
    let hf_wad: i128 = 1_020_000_000_000_000_000;
    let target_hf: i128 = 1_020_000_000_000_000_000;
    cvlr_satisfy!(hf_wad >= target_hf);
}

// ---------------------------------------------------------------------------
// Rule 9: bad_debt_at_exactly_5_usd -- DELETED (effectively tautological).
// Despite the production-call ceremony (`total_collateral_in_usd`,
// `total_borrow_in_usd`), the asserted predicate
// `total_debt > total_collateral && total_collateral <= 5*WAD` is a direct
// restatement of the two `cvlr_assume!`s. No production logic is exercised.
// Reinstate by invoking `clean_bad_debt_standalone` and observing the outcome.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn bad_debt_at_exactly_5_usd_sanity() {
    let total_collateral_usd = 5 * WAD;
    let bad_debt_threshold = 5 * WAD;
    cvlr_satisfy!(total_collateral_usd <= bad_debt_threshold);
}

// ---------------------------------------------------------------------------
// Rule 10: bad_debt_at_6_usd -- DELETED (effectively tautological).
// Same shape as Rule 9: predicate is restated from the cvlr_assume.
// Reinstate by invoking `clean_bad_debt_standalone` and observing the outcome.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn bad_debt_at_6_usd_sanity() {
    let total_collateral_usd = 6 * WAD;
    let bad_debt_threshold = 5 * WAD;
    cvlr_satisfy!(total_collateral_usd > bad_debt_threshold);
}

// ===========================================================================
// Precision Boundary Tests
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 11: mul_at_max_i128
// mul_half_up(i128::MAX / RAY, RAY, RAY) must not overflow.
// The I256 intermediate handles the large product safely.
// ---------------------------------------------------------------------------

#[rule]
fn mul_at_max_i128(e: Env) {
    // i128::MAX / RAY = 170_141_183_460_469_231_731 (approx 1.7e20)
    // This times RAY = i128::MAX (approx), which fits i128 after dividing by RAY.
    let a = i128::MAX / RAY;
    let b = RAY;

    let result = mul_div_half_up(&e, a, b, RAY);

    // a * RAY / RAY = a (within rounding)
    cvlr_assert!(result >= a - 1 && result <= a + 1);
}

#[rule]
fn mul_at_max_i128_sanity(e: Env) {
    let a = i128::MAX / RAY;
    let result = mul_div_half_up(&e, a, RAY, RAY);
    cvlr_satisfy!(result > 0);
}

// ---------------------------------------------------------------------------
// Rule 12: compound_taylor_accuracy
// For rate * time < 0.01 RAY (1% per period), the Taylor 5-term approximation
// error must be < RAY / 10000 (0.01% = 1 basis point).
// This rule uses 1% APY over 1 year: e^0.01 ~= 1.01005017...
// ---------------------------------------------------------------------------

#[rule]
fn compound_taylor_accuracy(e: Env) {
    // 1% annual rate -> per-ms rate
    let annual_rate_ray = RAY / 100; // 1%
    let rate_per_ms = div_by_int_half_up(annual_rate_ray, MILLISECONDS_PER_YEAR as i128);

    let factor = compound_interest(&e, Ray::from_raw(rate_per_ms), MILLISECONDS_PER_YEAR);

    // e^0.01 = 1.01005016708... in RAY
    // Expected: 1_010_050_167_084_168_058_000_000_000 (approx)
    // The tolerance is 0.01% = RAY / 10000
    let tolerance = RAY / 10_000;

    // Factor must be > 1.0 RAY (positive interest)
    cvlr_assert!(factor.raw() > RAY);

    // The 5-term Taylor expansion is highly accurate for small x.
    // Since exact `e^x` is not computed in the rule, check structural bounds:
    // Lower bound: 1 + x = 1.01 RAY
    // Upper bound: 1 + x + x^2/2 + ... < 1.0101 RAY (for x=0.01)
    let lower = RAY + annual_rate_ray; // 1.01 RAY
    cvlr_assert!(factor.raw() >= lower);
    cvlr_assert!(factor.raw() < lower + tolerance); // within 0.01% of 1.01
}

#[rule]
fn compound_taylor_accuracy_sanity(e: Env) {
    let rate_per_ms = div_by_int_half_up(RAY / 100, MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&e, Ray::from_raw(rate_per_ms), MILLISECONDS_PER_YEAR);
    // e^0.01 ~= 1.01005, must be between 1.01 and 1.0101
    cvlr_satisfy!(factor.raw() > RAY + RAY / 100);
}

// ---------------------------------------------------------------------------
// Rule 13: rescale_ray_to_wad
// rescale(RAY, 27, 18) must equal WAD (basic precision conversion).
// Downscaling by 9 decimals: (RAY + 10^9/2) / 10^9 = 10^18 = WAD.
// ---------------------------------------------------------------------------

#[rule]
fn rescale_ray_to_wad() {
    let result = rescale_half_up(RAY, 27, 18);
    cvlr_assert!(result == WAD);
}

// ---------------------------------------------------------------------------
// Rule 14: rescale_wad_to_7_decimals
// rescale(WAD, 18, 7) must equal 10^7 (Stellar native 7-decimal tokens).
// Downscaling by 11 decimals: (10^18 + 10^11/2) / 10^11 = 10^7.
// ---------------------------------------------------------------------------

#[rule]
fn rescale_wad_to_7_decimals() {
    let result = rescale_half_up(WAD, 18, 7);
    let expected = 10_000_000i128; // 10^7
    cvlr_assert!(result == expected);
}

// ===========================================================================
// Oracle Boundary Tests
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 15: tolerance_at_exact_first_bound -- DELETED (pure tautology).
// `cvlr_assume!(deviation == first_tolerance)` then asserting
// `deviation <= first_tolerance` is assumption-implies-assertion. The rule
// never invokes `oracle::is_within_anchor`. Reinstate by calling production
// oracle-tier discrimination and observing the chosen tier.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn tolerance_at_exact_first_bound_sanity() {
    let first_tolerance: i128 = 200; // 2%
    let deviation: i128 = 200;
    cvlr_satisfy!(deviation <= first_tolerance);
}

// ---------------------------------------------------------------------------
// Rule 16: tolerance_at_exact_second_bound -- DELETED (pure tautology).
// `deviation == second_tolerance` together with the prior assumption
// `second_tolerance > first_tolerance` makes the asserted predicate a direct
// consequence. No oracle production is invoked.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn tolerance_at_exact_second_bound_sanity() {
    let first_tolerance: i128 = 200;
    let second_tolerance: i128 = 500;
    let deviation: i128 = 500;
    cvlr_satisfy!(deviation > first_tolerance && deviation <= second_tolerance);
}

// ---------------------------------------------------------------------------
// Rule 17: tolerance_just_beyond_second -- DELETED (pure tautology).
// `deviation == second_tolerance + 1` directly implies the asserted
// `deviation > second_tolerance`. No oracle production is invoked.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn tolerance_just_beyond_second_sanity() {
    let second_tolerance: i128 = 500;
    let deviation: i128 = 501;
    cvlr_satisfy!(deviation > second_tolerance);
}

// ===========================================================================
// Position Boundary Tests
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 18: supply_dust_amount -- DELETED (subsumed).
// `mul_div_half_up(_, 1, RAY, RAY)` is the `a = 1` case of
// `math_rules::mul_half_up_identity`, which proves the identity over nondet
// `a` and strictly dominates this fixed point.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn supply_dust_amount_sanity(e: Env) {
    let scaled = mul_div_half_up(&e, 1, RAY, RAY);
    cvlr_satisfy!(scaled == 1);
}

// ---------------------------------------------------------------------------
// Rule 19: borrow_exact_reserves -- DELETED (pure tautology).
// `cvlr_assume!(borrow_amount == available_reserves)` then asserting
// `!(borrow_amount > available_reserves)` is assumption-implies-assertion.
// `pool::has_reserves` is never invoked. Reinstate by calling
// `compat::borrow_single` with `amount == available_reserves` (after wiring
// the pool summaries) and asserting the call does not panic.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn borrow_exact_reserves_sanity() {
    let reserves: i128 = 1_000_000;
    let borrow: i128 = 1_000_000;
    cvlr_satisfy!(borrow <= reserves);
}

// ---------------------------------------------------------------------------
// Rule 20: withdraw_more_than_position -- DELETED (pure tautology over `min`).
// Given `requested > position_value`, `requested.min(position_value)` is
// `position_value` by the definition of `min`. The rule never invokes the
// production withdraw path. Reinstate by calling `compat::withdraw_single`
// with `amount > position_value` (after wiring the pool summaries) and
// asserting the realised withdrawal equals `position_value`.
// Sanity twin retained below as a reachability check.
// ---------------------------------------------------------------------------

#[rule]
fn withdraw_more_than_position_sanity() {
    let position_value: i128 = 100;
    let requested: i128 = 200;
    let actual = requested.min(position_value);
    cvlr_satisfy!(actual == position_value);
}
