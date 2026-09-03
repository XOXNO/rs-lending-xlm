use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::Env;

use common::constants::{MAX_REASONABLE_PRICE_WAD, WAD};
use common::math::fp::Wad;
use common::math::fp_core::mul_div_half_up;

/// Production `read_scaled` computes the scaled price as
/// `factor.try_mul(quote)` (Wad), which is `fp_core::try_mul_div_half_up(factor,
/// quote, WAD)`: half-up rounding, `None` on non-positive operands or i128
/// overflow. These rules pin that semantics on a pure-math level (no host
/// storage), mirroring the pool/common split: the interesting arithmetic is
/// verified without resolving any feeds.
///
/// Both operands are capped at the registry's own ceiling,
/// `MAX_REASONABLE_PRICE_WAD` (1e9 WAD).
///
/// The quote leg is a resolved price, which `validate_sanity_bounds` holds at
/// or below that constant. The factor leg is clamped by the source's
/// `[min_factor_wad, max_factor_wad]` band, and `validation::factor_bounds`
/// rejects a `max_factor_wad` above the same constant. The worst-case biased
/// product is `1e27 * 1e27 = 1e54`, which the widened `I256` path handles and
/// which lands at `1e36` after the divide -- inside `i128`.
const MAX_FACTOR_WAD: i128 = MAX_REASONABLE_PRICE_WAD;
const MAX_QUOTE_WAD: i128 = MAX_REASONABLE_PRICE_WAD;

/// Largest `factor` for which `try_mul_div_half_up(factor, quote, WAD)` stays on
/// `fp_core`'s native `i128` fast path.
///
/// The branch condition is `factor * quote + WAD / 2 <= i128::MAX`, which for a
/// positive `quote` is exactly `factor <= (i128::MAX - WAD / 2) / quote`: below
/// the bound the product is at most `i128::MAX - WAD / 2`, and one above it the
/// product already exceeds that. `quote.max(1)` keeps the expression total; the
/// rules below all assume `quote > 0`, so the clamp never binds.
fn scaled_native_max(quote: i128) -> i128 {
    (i128::MAX - WAD / 2) / quote.max(1)
}

/// Native half of `scaled_price_pins_half_up_rounding`: the biased product fits
/// `i128`, so both the wrapped and the raw multiply run entirely in `i128`.
///
/// Lemma split of the former `scaled_price_pins_half_up_rounding`. The two
/// bounds are exact complements, so the pair covers the original
/// `(0, MAX_FACTOR_WAD] x (0, MAX_QUOTE_WAD]` box and each half asserts the
/// original identity.
#[rule]
fn scaled_price_pins_half_up_rounding_native(e: Env, factor: i128, quote: i128) {
    cvlr_assume!(factor > 0 && factor <= MAX_FACTOR_WAD);
    cvlr_assume!(quote > 0 && quote <= MAX_QUOTE_WAD);
    cvlr_assume!(factor <= scaled_native_max(quote));

    let scaled_price = Wad::from(factor)
        .try_mul(&e, Wad::from(quote))
        .unwrap()
        .raw();

    // Result uses half-up rounding on the WAD product (a switch to floor or
    // truncation changes this identity). The product is never negative for
    // positive operands, but it CAN round down to zero: `read_scaled`
    // (engine.rs) performs no post-multiplication positivity check, so a dust
    // factor*quote < WAD/2 yields an accepted zero price. Strict positivity
    // is enforced by production's feed-side checks, not by this math.
    let expected = mul_div_half_up(&e, factor, quote, WAD);
    cvlr_assert!(scaled_price == expected);
    cvlr_assert!(scaled_price >= 0);
}

/// Widened half of `scaled_price_pins_half_up_rounding`: the biased product
/// overflows `i128`, so both multiplies go through the exact `I256` host path.
#[rule]
fn scaled_price_pins_half_up_rounding_widened(e: Env, factor: i128, quote: i128) {
    cvlr_assume!(factor > 0 && factor <= MAX_FACTOR_WAD);
    cvlr_assume!(quote > 0 && quote <= MAX_QUOTE_WAD);
    cvlr_assume!(factor > scaled_native_max(quote));

    let scaled_price = Wad::from(factor)
        .try_mul(&e, Wad::from(quote))
        .unwrap()
        .raw();

    let expected = mul_div_half_up(&e, factor, quote, WAD);
    cvlr_assert!(scaled_price == expected);
    cvlr_assert!(scaled_price >= 0);
}

/// No lemma split and no upper bound on the operands: `try_mul_div_half_up`
/// rejects a negative `x` or `y` on its first line, before any multiply, so
/// neither branch is reached and there is no nonlinear term to bound.
#[rule]
fn scaled_price_rejects_negative_operands(e: Env, factor: i128, quote: i128) {
    cvlr_assume!(factor < 0 || quote < 0);

    let scaled = Wad::from(factor).try_mul(&e, Wad::from(quote));

    cvlr_assert!(scaled.is_none());
}

/// No lemma split: one operand is zero, so `checked_mul` returns zero and the
/// native branch always wins. The non-zero operand is bounded anyway, because
/// it is still a multiply operand and an unbounded one would make the branch
/// condition a full-range nonlinear query for no gain.
#[rule]
fn scaled_price_allows_zero_operands(e: Env, factor: i128, quote: i128) {
    cvlr_assume!(factor == 0 || quote == 0);
    cvlr_assume!((0..=MAX_FACTOR_WAD).contains(&factor));
    cvlr_assume!((0..=MAX_QUOTE_WAD).contains(&quote));

    // Zero is not a rejection cause in the wrapped multiplication (feeds are
    // filtered upstream); pin that a zero leg scales to a zero price.
    let scaled = Wad::from(factor).try_mul(&e, Wad::from(quote));

    cvlr_assert!(scaled.unwrap().raw() == 0);
}

/// Native half of `scaled_price_bounded_by_factor_clamp`.
///
/// The rule runs three products against the same `quote`:
/// `factor * quote`, `min_factor * quote` and `max_factor * quote`. They are
/// ordered by `min_factor <= factor <= max_factor`, so bounding the largest one
/// puts all three on the native path.
#[rule]
fn scaled_price_bounded_by_factor_clamp_native(
    e: Env,
    factor: i128,
    quote: i128,
    min_factor: i128,
    max_factor: i128,
) {
    cvlr_assume!(factor > 0 && factor <= MAX_FACTOR_WAD);
    cvlr_assume!(quote > 0 && quote <= MAX_QUOTE_WAD);
    cvlr_assume!(min_factor > 0 && min_factor <= factor && factor <= max_factor);
    cvlr_assume!(max_factor <= MAX_FACTOR_WAD);
    cvlr_assume!(max_factor <= scaled_native_max(quote));

    let scaled_price = Wad::from(factor)
        .try_mul(&e, Wad::from(quote))
        .unwrap()
        .raw();

    // With the factor confined to [min_factor, max_factor], the scaled price
    // stays within the prices the quote would scale to at the band edges
    // (half-up rounding moves the result by at most half a quote unit).
    let lo = mul_div_half_up(&e, quote, min_factor, WAD);
    let hi = mul_div_half_up(&e, quote, max_factor, WAD);
    cvlr_assert!(scaled_price >= lo.saturating_sub(1));
    cvlr_assert!(scaled_price <= hi.saturating_add(1));
}

/// Widened half of `scaled_price_bounded_by_factor_clamp`: the `max_factor`
/// product overflows, so at least the upper edge is computed in `I256`. The
/// lower edge may still be native, which is the branch mix this lemma is for;
/// the negated bound makes the two domains a partition of the original.
#[rule]
fn scaled_price_bounded_by_factor_clamp_widened(
    e: Env,
    factor: i128,
    quote: i128,
    min_factor: i128,
    max_factor: i128,
) {
    cvlr_assume!(factor > 0 && factor <= MAX_FACTOR_WAD);
    cvlr_assume!(quote > 0 && quote <= MAX_QUOTE_WAD);
    cvlr_assume!(min_factor > 0 && min_factor <= factor && factor <= max_factor);
    cvlr_assume!(max_factor <= MAX_FACTOR_WAD);
    cvlr_assume!(max_factor > scaled_native_max(quote));

    let scaled_price = Wad::from(factor)
        .try_mul(&e, Wad::from(quote))
        .unwrap()
        .raw();

    let lo = mul_div_half_up(&e, quote, min_factor, WAD);
    let hi = mul_div_half_up(&e, quote, max_factor, WAD);
    cvlr_assert!(scaled_price >= lo.saturating_sub(1));
    cvlr_assert!(scaled_price <= hi.saturating_add(1));
}

/// Native half of `scaled_price_monotone_in_factor`: split on the larger
/// factor, since `factor1 <= factor2` implies `factor1 * quote <= factor2 *
/// quote`, so bounding `factor2` puts both multiplies on the native path.
#[rule]
fn scaled_price_monotone_in_factor_native(e: Env, factor1: i128, factor2: i128, quote: i128) {
    cvlr_assume!(factor1 > 0 && factor1 <= factor2);
    cvlr_assume!(factor2 <= MAX_FACTOR_WAD);
    cvlr_assume!(quote > 0 && quote <= MAX_QUOTE_WAD);
    cvlr_assume!(factor2 <= scaled_native_max(quote));

    let scaled1 = Wad::from(factor1).try_mul(&e, Wad::from(quote));
    let scaled2 = Wad::from(factor2).try_mul(&e, Wad::from(quote));

    cvlr_assert!(scaled1.unwrap().raw() <= scaled2.unwrap().raw());
}

/// Widened half of `scaled_price_monotone_in_factor`: the `factor2` product
/// overflows `i128`. Exact complement of the native lemma's bound, so the pair
/// covers the original domain.
#[rule]
fn scaled_price_monotone_in_factor_widened(e: Env, factor1: i128, factor2: i128, quote: i128) {
    cvlr_assume!(factor1 > 0 && factor1 <= factor2);
    cvlr_assume!(factor2 <= MAX_FACTOR_WAD);
    cvlr_assume!(quote > 0 && quote <= MAX_QUOTE_WAD);
    cvlr_assume!(factor2 > scaled_native_max(quote));

    let scaled1 = Wad::from(factor1).try_mul(&e, Wad::from(quote));
    let scaled2 = Wad::from(factor2).try_mul(&e, Wad::from(quote));

    cvlr_assert!(scaled1.unwrap().raw() <= scaled2.unwrap().raw());
}

/// Deliberately **not** lemma-split: the property under proof is exactly that
/// neither branch fails on a realistic feed. Assuming the native branch
/// condition would make the `is_some()` assertion follow from the assumption,
/// and assuming its negation would drop the half the assertion is cheapest on.
/// The worst-case product is (1e9 WAD)^2 = 1e54, so the widened `I256` path is
/// the one that has to produce a value inside `i128` -- 1e36 after the divide,
/// two orders of magnitude below `i128::MAX`.
#[rule]
fn scaled_price_fits_i128_within_price_bounds(e: Env, factor: i128, quote: i128) {
    cvlr_assume!(factor > 0 && factor <= MAX_FACTOR_WAD);
    cvlr_assume!(quote > 0 && quote <= MAX_QUOTE_WAD);

    let scaled = Wad::from(factor).try_mul(&e, Wad::from(quote));
    cvlr_assert!(scaled.is_some());
}
