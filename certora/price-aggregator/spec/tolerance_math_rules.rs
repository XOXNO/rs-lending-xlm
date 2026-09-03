use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{panic_with_error, Env};

use common::constants::{BPS, RAY, WAD};
use common::errors::OracleError;
use common::math::fp_core;

use common::types::OracleTolerance;

/// Ceiling on a WAD-scaled oracle price used by every rule in this module.
///
/// `MAX_REASONABLE_PRICE_WAD` (`common/src/constants/shared.rs`) is the highest
/// price `validate_sanity_bounds` accepts, at `1e9 * WAD`. This module works one
/// thousand times below that: the most expensive listed asset is BTC at ~1.2e5
/// USD per whole token (`docs/reference/numeric-bounds.md` §6.2), so `1e6 * WAD`
/// leaves an order of magnitude of headroom over anything the registry can
/// hold. The bound excludes prices above one million USD per whole token, which
/// no configured sanity band admits.
const MAX_PRICE_WAD: i128 = 1_000_000 * WAD;

/// Largest `price` for which `mul_div_half_up(price, RAY, price)` stays on
/// `fp_core`'s native `i128` fast path.
///
/// The branch condition is `price * RAY + price / 2 <= i128::MAX`. That is
/// exactly `price <= i128::MAX / RAY`: at the bound `price * RAY` leaves the
/// remainder `i128::MAX mod RAY = 4.69e26`, which dwarfs `price / 2 = 8.5e10`,
/// and one above it `price * RAY` already exceeds `i128::MAX`.
const PAR_RATIO_NATIVE_MAX: i128 = i128::MAX / RAY;

fn midpoint_if_in_band(e: &Env, anchor: i128, primary: i128, tolerance: &OracleTolerance) -> i128 {
    if !crate::tolerance::within_tolerance_band(e, anchor, primary, tolerance) {
        panic_with_error!(e, OracleError::UnsafePriceNotAllowed);
    }
    crate::tolerance::midpoint_price_or_zero(anchor, primary)
}

/// A zero anchor is rejected: `within_tolerance_band` divides by the smaller of
/// the two prices, so a zero leg can never be blended in.
///
/// Revert shape: the trailing assert is reachable only if `midpoint_if_in_band`
/// returns. Paired with `zero_anchor_reverts_fixture_completes`, which drives
/// the same tolerance with a positive anchor and completes.
#[rule]
fn zero_anchor_reverts(e: Env, anchor: i128, primary: i128) {
    cvlr_assume!(anchor == 0);
    cvlr_assume!(primary > 0 && primary <= MAX_PRICE_WAD);
    let tolerance = OracleTolerance {
        upper_ratio_bps: 20_000,
        lower_ratio_bps: 1,
    };
    let _ = midpoint_if_in_band(&e, anchor, primary, &tolerance);
    cvlr_assert!(false);
}

/// Satisfy twin of [`zero_anchor_reverts`]: the same tolerance and the same
/// price domain, with the gate condition flipped from `anchor == 0` to a
/// positive anchor that sits at par with the primary (ratio 10_000, inside the
/// 20_000 upper bound). The witness proves the revert rule's fixture is not
/// unsatisfiable for a reason other than the zero anchor.
#[rule]
fn zero_anchor_reverts_fixture_completes(e: Env, anchor: i128, primary: i128) {
    cvlr_assume!(anchor == primary);
    cvlr_assume!(primary > 0 && primary <= MAX_PRICE_WAD);
    let tolerance = OracleTolerance {
        upper_ratio_bps: 20_000,
        lower_ratio_bps: 1,
    };
    let final_price = midpoint_if_in_band(&e, anchor, primary, &tolerance);
    cvlr_satisfy!(final_price == primary);
}

/// No lemma split: `within_tolerance_band` computes
/// `try_mul_div_half_up(high, BPS, low)`, whose product is at most
/// `MAX_PRICE_WAD * BPS = 1e28`, five orders of magnitude below `i128::MAX`.
/// The widened branch is unreachable on this domain.
#[rule]
fn equal_prices_within_symmetric_band(e: Env, price: i128) {
    cvlr_assume!(price > 0 && price <= MAX_PRICE_WAD);

    let tolerance = OracleTolerance {
        upper_ratio_bps: 10_200,
        lower_ratio_bps: 9_800,
    };
    let final_price = midpoint_if_in_band(&e, price, price, &tolerance);
    cvlr_assert!(final_price == price);
}

/// Native half of `par_ratio_is_bps`: `price * RAY + price / 2` fits `i128`, so
/// the ray ratio is computed entirely in `i128`.
///
/// Lemma split of the former `par_ratio_is_bps`. The two bounds are exact
/// complements, so the pair covers `(0, MAX_PRICE_WAD]` and each half asserts
/// the original identity. `rescale_half_up` divides by a power of ten and has
/// no branch of its own.
#[rule]
fn par_ratio_is_bps_native(e: Env, price: i128) {
    cvlr_assume!(price > 0 && price <= MAX_PRICE_WAD);
    cvlr_assume!(price <= PAR_RATIO_NATIVE_MAX);

    let ratio_ray = fp_core::mul_div_half_up(&e, price, RAY, price);
    let ratio_bps = fp_core::rescale_half_up(&e, ratio_ray, 27, 4);
    cvlr_assert!(ratio_bps == BPS);
}

/// Widened half of `par_ratio_is_bps`: the biased product overflows `i128`, so
/// the ray ratio is computed with exact `I256` host calls.
#[rule]
fn par_ratio_is_bps_widened(e: Env, price: i128) {
    cvlr_assume!(price > 0 && price <= MAX_PRICE_WAD);
    cvlr_assume!(price > PAR_RATIO_NATIVE_MAX);

    let ratio_ray = fp_core::mul_div_half_up(&e, price, RAY, price);
    let ratio_bps = fp_core::rescale_half_up(&e, ratio_ray, 27, 4);
    cvlr_assert!(ratio_bps == BPS);
}

/// Two prices a factor of two apart are outside a +/- 10 bps band.
///
/// Revert shape; paired with `divergent_prices_revert_fixture_completes`.
#[rule]
fn divergent_prices_revert(e: Env, anchor: i128, primary: i128) {
    cvlr_assume!(anchor > 0 && anchor <= MAX_PRICE_WAD);
    cvlr_assume!(primary == 2 * anchor);

    let tolerance = OracleTolerance {
        upper_ratio_bps: 10_010,
        lower_ratio_bps: 9_990,
    };
    let _ = midpoint_if_in_band(&e, anchor, primary, &tolerance);
    cvlr_assert!(false);
}

/// Satisfy twin of [`divergent_prices_revert`]: the same band and the same
/// anchor domain, with the divergence gate flipped from `primary == 2 * anchor`
/// (ratio 20_000) to `primary == anchor` (ratio 10_000, inside 9_990..=10_010).
#[rule]
fn divergent_prices_revert_fixture_completes(e: Env, anchor: i128, primary: i128) {
    cvlr_assume!(anchor > 0 && anchor <= MAX_PRICE_WAD);
    cvlr_assume!(primary == anchor);

    let tolerance = OracleTolerance {
        upper_ratio_bps: 10_010,
        lower_ratio_bps: 9_990,
    };
    let final_price = midpoint_if_in_band(&e, anchor, primary, &tolerance);
    cvlr_satisfy!(final_price == anchor);
}
