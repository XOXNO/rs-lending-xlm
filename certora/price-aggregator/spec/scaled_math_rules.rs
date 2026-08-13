use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::Env;

/// Production `read_scaled` computes the scaled price as
/// `factor.try_mul(quote)` (Wad), which is `fp_core::try_mul_div_half_up(factor,
/// quote, WAD)`: half-up rounding, `None` on non-positive operands or i128
/// overflow. These rules pin that semantics on a pure-math level (no host
/// storage), mirroring the pool/common split: the interesting arithmetic is
/// verified without resolving any feeds.
const MAX_FACTOR_WAD: i128 = 10_000 * common::constants::WAD;
const MAX_QUOTE_WAD: i128 = 10_000 * common::constants::WAD;

#[rule]
fn scaled_price_pins_half_up_rounding(e: Env, factor: i128, quote: i128) {
    cvlr_assume!(factor > 0 && factor <= MAX_FACTOR_WAD);
    cvlr_assume!(quote > 0 && quote <= MAX_QUOTE_WAD);

    let scaled =
        common::math::fp::Wad::from(factor).try_mul(&e, common::math::fp::Wad::from(quote));
    let scaled_price = scaled.unwrap().raw();

    // Result uses half-up rounding on the WAD product (a switch to floor or
    // truncation changes this identity). The product is never negative for
    // positive operands, but it CAN round down to zero: `read_scaled`
    // (engine.rs) performs no post-multiplication positivity check, so a dust
    // factor*quote < WAD/2 yields an accepted zero price. Strict positivity
    // is enforced by production's feed-side checks, not by this math.
    let expected =
        common::math::fp_core::mul_div_half_up(&e, factor, quote, common::constants::WAD);
    cvlr_assert!(scaled_price == expected);
    cvlr_assert!(scaled_price >= 0);
}

#[rule]
fn scaled_price_rejects_negative_operands(e: Env, factor: i128, quote: i128) {
    cvlr_assume!(factor < 0 || quote < 0);

    let scaled =
        common::math::fp::Wad::from(factor).try_mul(&e, common::math::fp::Wad::from(quote));

    cvlr_assert!(scaled.is_none());
}

#[rule]
fn scaled_price_allows_zero_operands(e: Env, factor: i128, quote: i128) {
    cvlr_assume!(factor == 0 || quote == 0);
    cvlr_assume!(factor >= 0 && quote >= 0);

    // Zero is not a rejection cause in the wrapped multiplication (feeds are
    // filtered upstream); pin that a zero leg scales to a zero price.
    let scaled =
        common::math::fp::Wad::from(factor).try_mul(&e, common::math::fp::Wad::from(quote));

    cvlr_assert!(scaled.unwrap().raw() == 0);
}
#[rule]
fn scaled_price_bounded_by_factor_clamp(
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

    let scaled =
        common::math::fp::Wad::from(factor).try_mul(&e, common::math::fp::Wad::from(quote));
    let scaled_price = scaled.unwrap().raw();

    // With the factor confined to [min_factor, max_factor], the scaled price
    // stays within the prices the quote would scale to at the band edges
    // (half-up rounding moves the result by at most half a quote unit).
    let lo = common::math::fp_core::mul_div_half_up(&e, quote, min_factor, common::constants::WAD);
    let hi = common::math::fp_core::mul_div_half_up(&e, quote, max_factor, common::constants::WAD);
    cvlr_assert!(scaled_price >= lo.saturating_sub(1));
    cvlr_assert!(scaled_price <= hi.saturating_add(1));
}

#[rule]
fn scaled_price_monotone_in_factor(e: Env, factor1: i128, factor2: i128, quote: i128) {
    cvlr_assume!(factor1 > 0 && factor1 <= factor2);
    cvlr_assume!(factor2 <= MAX_FACTOR_WAD);
    cvlr_assume!(quote > 0 && quote <= MAX_QUOTE_WAD);

    let scaled1 =
        common::math::fp::Wad::from(factor1).try_mul(&e, common::math::fp::Wad::from(quote));
    let scaled2 =
        common::math::fp::Wad::from(factor2).try_mul(&e, common::math::fp::Wad::from(quote));

    cvlr_assert!(scaled1.unwrap().raw() <= scaled2.unwrap().raw());
}

#[rule]
fn scaled_price_fits_i128_within_price_bounds(e: Env, factor: i128, quote: i128) {
    cvlr_assume!(factor > 0 && factor <= MAX_FACTOR_WAD);
    cvlr_assume!(quote > 0 && quote <= MAX_QUOTE_WAD);

    // The worst-case product is (10^4 WAD)^2 / WAD = 10^26, far inside i128;
    // the wrapped multiplication must never fail on realistic feeds.
    let scaled =
        common::math::fp::Wad::from(factor).try_mul(&e, common::math::fp::Wad::from(quote));
    cvlr_assert!(scaled.is_some());
}
