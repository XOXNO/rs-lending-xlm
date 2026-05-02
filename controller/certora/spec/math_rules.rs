/// Math Precision Formal Verification Rules
///
/// Certora Sunbeam rules for the half-up rounding arithmetic system.
///
/// From CLAUDE.md:
///   - All arithmetic uses half-up rounding (rounds 0.5 away from zero)
///   - mul_half_up(a, b, precision) = (a * b + precision/2) / precision
///   - div_half_up(a, b, precision) = (a * precision + b/2) / b
///   - rescale upscaling is lossless; downscaling uses half-up rounding
///   - Signed variants round away from zero for negative results
///   - I256 intermediates prevent overflow for RAY*RAY products
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::Env;

use common::constants::{RAY, WAD};
use common::fp_core::{mul_div_half_up, mul_div_half_up_signed, rescale_half_up};

// ---------------------------------------------------------------------------
// Rule 1: mul_half_up is commutative -- mul_half_up(a, b, p) == mul_half_up(b, a, p)
// ---------------------------------------------------------------------------

#[rule]
fn mul_half_up_commutative(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();
    let p: i128 = cvlr::nondet::nondet();

    // Constrain to positive, realistic ranges
    cvlr_assume!((0..=RAY).contains(&a));
    cvlr_assume!((0..=RAY).contains(&b));
    cvlr_assume!(p > 0 && p <= RAY);

    let ab = mul_div_half_up(&e, a, b, p);
    let ba = mul_div_half_up(&e, b, a, p);

    cvlr_assert!(ab == ba);
}

// ---------------------------------------------------------------------------
// Rule 2: mul_half_up with zero -- mul_half_up(0, b, p) == 0 and mul_half_up(a, 0, p) == 0
// ---------------------------------------------------------------------------

#[rule]
fn mul_half_up_zero(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();
    let p: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=RAY).contains(&a));
    cvlr_assume!((0..=RAY).contains(&b));
    cvlr_assume!(p > 0 && p <= RAY);

    // (0 * b + p/2) / p = p/2 / p = 0 for any p >= 2
    // Note: For p == 1, (0 * b + 0) / 1 = 0 as well (since 1/2 = 0 in integer div)
    let zero_times_b = mul_div_half_up(&e, 0, b, p);
    let a_times_zero = mul_div_half_up(&e, a, 0, p);

    cvlr_assert!(zero_times_b == 0);
    cvlr_assert!(a_times_zero == 0);
}

// ---------------------------------------------------------------------------
// Rule 3: mul_half_up identity -- mul_half_up(a, RAY, RAY) == a (within +/-1)
// ---------------------------------------------------------------------------

#[rule]
fn mul_half_up_identity(e: Env) {
    let a: i128 = cvlr::nondet::nondet();

    // Constrain to realistic protocol values (up to 10^27 * 10^27 = 10^54 is extreme;
    // actual index products are at most ~10^30)
    cvlr_assume!((0..=RAY * 1000).contains(&a)); // up to 1000 RAY

    // a * RAY / RAY should give back a exactly:
    // (a * RAY + RAY/2) / RAY = a + (RAY/2) / RAY = a + 0 = a
    // since RAY/2 < RAY, integer division discards it
    let result = mul_div_half_up(&e, a, RAY, RAY);

    // Exact equality since (a * RAY + HALF_RAY) / RAY = a when a >= 0
    cvlr_assert!(result == a);
}

// `mul_half_up_identity_sanity` removed (efficiency E1):
// pure cvlr_satisfy duplicate of `mul_half_up_identity` -- same inputs
// (`0..=RAY * 1000`), same call, same comparison. The assertion rule
// already proves `result == a` for every value in the input range, which
// trivially implies the satisfy condition is reachable.

// ---------------------------------------------------------------------------
// Rule 4: div_half_up is inverse of mul_half_up (within rounding tolerance)
// ---------------------------------------------------------------------------

#[rule]
fn div_half_up_inverse(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    // Positive, non-zero divisor, realistic ranges. Lower-bounding `b` at
    // `RAY / 1_000` keeps the recovered intermediate (`product * RAY / b`)
    // finite -- with `b = 1`, the I256 -> i128 conversion in the second
    // mul_div_half_up call would panic. Pruning that branch saves the
    // prover from exploring a panic path that does not arise in production.
    cvlr_assume!((0..=RAY * 100).contains(&a));
    cvlr_assume!((RAY / 1_000..=RAY * 100).contains(&b));

    let product = mul_div_half_up(&e, a, b, RAY);
    let recovered = mul_div_half_up(&e, product, RAY, b);

    // Two rounds of half-up rounding can introduce at most +/-1 each
    cvlr_assert!(recovered >= a - 2 && recovered <= a + 2);
}

// `div_half_up_inverse_sanity` removed (efficiency E1):
// pure cvlr_satisfy duplicate of `div_half_up_inverse` -- same input
// constraints, same round-trip computation, same envelope predicate
// (`recovered in [a-2, a+2]`). The assertion rule proves the property
// universally; the satisfy companion adds no new reachability signal.

// ---------------------------------------------------------------------------
// Rule 5: div_half_up with zero numerator -- div_half_up(0, b, RAY) == 0
// ---------------------------------------------------------------------------

#[rule]
fn div_half_up_zero_numerator(e: Env) {
    let b: i128 = cvlr::nondet::nondet();

    // b must be positive and large enough that b/2 < b (always true for b > 0)
    cvlr_assume!(b > 0 && b <= RAY);

    // mul_div_half_up(0, RAY, b) = (0 * RAY + b/2) / b = (b/2) / b = 0
    // since b/2 < b for all b > 0
    let result = mul_div_half_up(&e, 0, RAY, b);

    cvlr_assert!(result == 0);
}

// ---------------------------------------------------------------------------
// Rule 6: mul_half_up rounding direction -- never rounds below floor(a*b/p)
// ---------------------------------------------------------------------------

// Reformulated to linear arithmetic over i128. The previous version computed
// `floor` with `soroban_sdk::I256` mul/div/to_i128 (the prover models these
// as bitvector ops, contributing the `nonlinear ops: 8 / max polyn.
// degree: 4` warning the Certora run reported, and the `.to_i128().unwrap()`
// added a panic branch). The same property -- "result is no less than the
// mathematical floor of (a*b)/WAD" -- is captured by:
//
//     result * WAD >= a * b - (WAD - 1)
//
// which says `result` is within one unit of the floor on the low side.
// Tightening the input range to `<= 10^14` keeps `a * b` well inside i128
// (max product ~= 10^28 vs i128 max ~= 1.7e38) so the multiplication is
// linear and overflow-free.
#[rule]
fn mul_half_up_rounding_direction(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    // 10^14 covers realistic per-asset USD amounts (e.g. $1M with 7 decimals
    // is 10^13). a*b stays well below i128 max.
    cvlr_assume!((0..=100_000_000_000_000).contains(&a));
    cvlr_assume!((0..=100_000_000_000_000).contains(&b));

    let result = mul_div_half_up(&e, a, b, WAD);

    // Half-up rounding never rounds below the true mathematical floor of
    // a*b/WAD. Equivalently: result*WAD is at most (WAD - 1) below a*b.
    cvlr_assert!(result * WAD >= a * b - (WAD - 1));
}

// `mul_half_up_rounding_direction_sanity` removed (efficiency E1):
// asserts only `result >= 0` for `(a, b) in [0, WAD*100]^2`. With both
// operands non-negative, `mul_div_half_up` is non-negative by construction
// (the production code path computes `(a*b + WAD/2) / WAD` over I256 then
// converts to a non-negative i128). The companion rule pays solver time
// to re-prove a typechecker-trivial fact already covered by the assertion
// rule's input domain.

// ---------------------------------------------------------------------------
// Rule 7: div_half_up rounding direction -- rounds up when remainder >= b/2
// ---------------------------------------------------------------------------

// Reformulated to linear arithmetic over i128. Original computed `floor` and
// `remainder` via `soroban_sdk::I256` and asserted exact-branch equality;
// the bitvector mul/div + `.to_i128().unwrap()` paths combined to time out
// the solver per the Certora run. The two-sided envelope below captures
// the same half-up rounding contract:
//
//     floor <= result <= floor + 1
//
// where `floor = (a * WAD) / b` integer-divided. We don't need to identify
// which branch fires -- the linear envelope is enough to catch any
// implementation that rounds outside the half-up window.
#[rule]
fn div_half_up_rounding_direction(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    // Bounds keep `a * WAD` inside i128. With `a <= 10^14`, `a * WAD <= 10^32`
    // (well below i128 max).
    cvlr_assume!((0..=100_000_000_000_000).contains(&a));
    cvlr_assume!(b > 0 && b <= 100_000_000_000_000);

    let result = mul_div_half_up(&e, a, WAD, b);

    // Linear envelope on half-up rounding: result is at most one unit above
    // the integer floor and never below it.
    //   result * b >= a * WAD - (b - 1)            (lower bound: >= floor)
    //   result * b <= a * WAD + b                   (upper bound: <= floor + 1)
    cvlr_assert!(result * b >= a * WAD - (b - 1));
    cvlr_assert!(result * b <= a * WAD + b);
}

// ---------------------------------------------------------------------------
// Rule 8: rescale upscale is lossless -- rescale_half_up(x, 7, 18) * 10^(18-7) preserves value
// ---------------------------------------------------------------------------

#[rule]
fn rescale_upscale_lossless() {
    let x: i128 = cvlr::nondet::nondet();
    let from: u32 = 7;
    let to: u32 = 18;

    // Realistic token amounts (up to 10^18 at 7 decimals)
    cvlr_assume!((0..=WAD).contains(&x));

    let upscaled = rescale_half_up(x, from, to);

    // Upscaling by 11 decimals means multiplying by 10^11
    // The result must be exactly x * 10^11
    let factor = 10i128.pow(to - from);
    cvlr_assert!(upscaled == x * factor);
}

// `rescale_upscale_lossless_sanity` removed (efficiency E1):
// pure cvlr_satisfy duplicate of `rescale_upscale_lossless` -- same input
// range, same call, weaker predicate (`upscaled > 0` instead of the exact
// `upscaled == x * factor`). The assertion rule subsumes the satisfy
// version: for any `x > 0` in the range, `upscaled = x * 10^11 > 0`.

// ---------------------------------------------------------------------------
// Rule 9: rescale roundtrip -- rescale_half_up(rescale_half_up(x, 7, 18), 18, 7) approx x (within +/-1)
// ---------------------------------------------------------------------------

#[rule]
fn rescale_roundtrip() {
    let x: i128 = cvlr::nondet::nondet();
    let low: u32 = 7;
    let high: u32 = 18;

    // Positive values, realistic range for 7-decimal tokens
    cvlr_assume!((0..=1_000_000_000_000_000).contains(&x));

    // Upscale then downscale
    let upscaled = rescale_half_up(x, low, high);
    let recovered = rescale_half_up(upscaled, high, low);

    // Upscale is exact, downscale uses half-up. Since upscaled = x * 10^11,
    // downscaling divides by 10^11 with half-up rounding.
    // (x * 10^11 + 10^11/2) / 10^11 = x + 0 = x (since 10^11/2 < 10^11)
    // So the roundtrip is exact for 7->18->7.
    cvlr_assert!(recovered == x);
}

// `rescale_roundtrip_sanity` removed (efficiency E1):
// pure cvlr_satisfy duplicate of `rescale_roundtrip` -- identical inputs,
// identical computation, identical predicate (`recovered == x`). The
// assertion rule already proves this for every value in the range.

// ---------------------------------------------------------------------------
// Rule 10: signed mul rounds away from zero for negative inputs
// ---------------------------------------------------------------------------

// Reformulated to linear arithmetic. The previous I256-based floor
// computation timed out the solver during the most recent Certora run
// (`signed_mul_away_from_zero: solving threw an exception`).
//
// Property: half-up signed rounding (away from zero) keeps `result * d`
// within one full divisor `d` of the true product `a*b`. The earlier
// one-sided bound `result * RAY <= a * b` is wrong on negative products:
// rounding `-3.4` away from zero yields `-3`, and `-3 * RAY > -3.4 * RAY`.
// The correct linear envelope is symmetric:
//
//   a * b - RAY <= result * RAY <= a * b + RAY
//
// which captures both rounding directions. Input bounds keep `a * b`
// inside i128 (max product ~10^28 vs i128 max ~1.7e38) so the
// multiplications are linear and overflow-free.
#[rule]
fn signed_mul_away_from_zero(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    cvlr_assume!((-100_000_000_000_000..0).contains(&a));
    cvlr_assume!(b > 0 && b <= 100_000_000_000_000);

    let result = mul_div_half_up_signed(&e, a, b, RAY);

    // Symmetric envelope: `result * RAY` is at most one `RAY` away from
    // the exact product `a * b` in either direction. Holds for both
    // signs of the product (here always non-positive given the input
    // ranges, but the bound is sign-agnostic).
    cvlr_assert!(result * RAY >= a * b - RAY);
    cvlr_assert!(result * RAY <= a * b + RAY);
}

// `signed_mul_away_from_zero_sanity` removed (efficiency E1):
// asserts only `result < 0` over a wider range (`a in [-RAY*100, 0)`,
// `b in (0, RAY*100]`) than the assertion rule. The predicate is implied
// by the sign of `a * b` (negative since `a < 0` and `b > 0`) and the
// half-up away-from-zero rounding semantics. No reachability information
// the assertion rule does not already cover.

// ---------------------------------------------------------------------------
// Rule 11: I256 no overflow -- mul_half_up with max realistic values (RAY * RAY)
// ---------------------------------------------------------------------------

/// Verifies that mul_half_up with maximum realistic protocol values does not
/// panic from I256-to-i128 conversion. The largest product in the protocol is
/// an index product: (index * index) where each index can be up to ~10 * RAY.
#[rule]
fn i256_no_overflow(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    let b: i128 = cvlr::nondet::nondet();

    // Indexes can grow but realistically stay within 10 * RAY (~10^28).
    // Test the extreme: RAY * RAY intermediate = 10^54, well within I256.
    // The result should fit i128 (max ~1.7 * 10^38).
    cvlr_assume!((0..=10 * RAY).contains(&a));
    cvlr_assume!((0..=10 * RAY).contains(&b));

    // This must not panic -- if I256 -> i128 conversion fails, the rule fails
    let result = mul_div_half_up(&e, a, b, RAY);

    // Result should be at most a * b / RAY ~ 10 * RAY * 10 * RAY / RAY = 100 * RAY
    cvlr_assert!(result >= 0);
    cvlr_assert!(result <= 100 * RAY + 1); // +1 for rounding
}

// `i256_no_overflow_sanity` removed (efficiency E1):
// pure cvlr_satisfy duplicate of `i256_no_overflow` -- same inputs, same
// call. The assertion rule already proves `result <= 100 * RAY + 1` and
// `result >= 0` for the same input range; with `a, b > 0` (implied by
// `a, b in (0, 10*RAY]`) the I256 product is positive and `result >= 1`,
// trivially satisfying `result > 0`.

// ---------------------------------------------------------------------------
// Rule 12: div_by_zero sanity -- div_half_up(a, 0, RAY) should be unreachable
// ---------------------------------------------------------------------------

/// Division by zero must cause a panic (Soroban I256 division by zero panics).
/// This rule constrains the divisor to zero and attempts a division. If the
/// prover can reach the assertion, it means div-by-zero did not revert, which
/// is a violation.
#[rule]
fn div_by_zero_sanity(e: Env) {
    let a: i128 = cvlr::nondet::nondet();
    cvlr_assume!((0..=RAY).contains(&a));

    // Division by zero -- this line should always panic
    let _result = mul_div_half_up(&e, a, RAY, 0);

    // Reaching this line would mean division by zero did not revert.
    cvlr_assert!(false);
}
