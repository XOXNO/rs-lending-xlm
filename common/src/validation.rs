//! Cross-contract guard checks shared by the pool and controller.
//!
//! Each guard panics with a stable protocol error so both contracts report
//! identical error codes for the same malformed input.

use crate::constants::{
    BPS, MAX_LIQUIDATION_TARGET_HF_WAD, MAX_REASONABLE_PRICE_WAD, MAX_TOLERANCE, MIN_TOLERANCE,
    RAY_DECIMALS, WAD,
};
use crate::errors::{CollateralError, FlashLoanError, GenericError, OracleError};
use crate::math::fp_core::mul_div_ceil;
use crate::oracle::observation::MAX_SINGLE_SOURCE_SANITY_BAND_BPS;
use crate::types::{OraclePriceFluctuation, OracleStrategy};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Executable};

/// Requires a strictly positive amount.
///
/// # Errors
/// * [`GenericError::AmountMustBePositive`] - `amount <= 0`.
pub fn require_positive_amount(env: &Env, amount: i128) {
    assert_with_error!(env, amount > 0, GenericError::AmountMustBePositive);
}

/// Requires a non-negative amount. Zero is accepted for flows where it carries a
/// sentinel meaning (withdraw-all, zero fee, zero rewards).
///
/// # Errors
/// * [`GenericError::AmountMustBePositive`] - `amount < 0`.
pub fn require_nonneg_amount(env: &Env, amount: i128) {
    assert_with_error!(env, amount >= 0, GenericError::AmountMustBePositive);
}

/// Caps of zero/negative or `i128::MAX` mean "no cap configured"; the
/// controller's spoke-cap enforcement and `max_*` previews share this rule.
pub fn cap_is_enabled(cap: i128) -> bool {
    cap > 0 && cap != i128::MAX
}

/// Requires a cap that fits asset-to-RAY scaling. Disabled sentinels (`0` and
/// `i128::MAX`) pass without a bound check.
///
/// # Errors
/// * [`CollateralError::AssetDecimalsTooHigh`] - `asset_decimals > RAY_DECIMALS`.
/// * [`CollateralError::InvalidBorrowParams`] - `cap` exceeds the asset-to-RAY
///   scaling ceiling.
pub fn require_cap_within_asset_domain(env: &Env, cap: i128, asset_decimals: u32) {
    if cap == i128::MAX {
        return;
    }
    let exp = RAY_DECIMALS
        .checked_sub(asset_decimals)
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::AssetDecimalsTooHigh));
    let cap_ceiling = i128::MAX
        / 10i128.checked_pow(exp).expect(
            "10^(RAY_DECIMALS - asset_decimals) fits i128 for asset_decimals <= RAY_DECIMALS",
        );
    assert_with_error!(
        env,
        cap <= cap_ceiling,
        CollateralError::InvalidBorrowParams
    );
}

/// Requires the flash-loan receiver to be a deployed Wasm contract.
///
/// # Errors
/// * [`FlashLoanError::InvalidFlashloanReceiver`] - `receiver` is not a deployed
///   Wasm contract.
pub fn require_wasm_receiver(env: &Env, receiver: &Address) {
    assert_with_error!(
        env,
        matches!(receiver.executable(), Some(Executable::Wasm(_))),
        FlashLoanError::InvalidFlashloanReceiver
    );
}

/// Requires the protocol liquidation fee to be at most 100% (`BPS`).
///
/// The fee is applied to the seized-collateral bonus at liquidation time; an
/// oversized value would make liquidation planning revert for the asset.
///
/// # Errors
/// * [`CollateralError::InvalidLiqThreshold`] - `fees_bps` exceeds `BPS` (100%).
pub fn validate_liquidation_fees(env: &Env, fees_bps: u32) {
    assert_with_error!(
        env,
        i128::from(fees_bps) <= BPS,
        CollateralError::InvalidLiqThreshold
    );
}

/// Validates loan-to-value, liquidation-threshold, and liquidation-bonus (bps);
/// governance and controller setters enforce the same bounds. Keeps the threshold
/// above the LTV and seizure within collateral backing.
///
/// # Errors
/// * [`CollateralError::InvalidLiqThreshold`] - `threshold <= ltv`,
///   `threshold > BPS`, or `threshold * (BPS + bonus) > BPS * BPS`.
pub fn validate_risk_bounds(env: &Env, ltv: u32, threshold: u32, bonus: u32) {
    let ltv = i128::from(ltv);
    let threshold = i128::from(threshold);
    let bonus = i128::from(bonus);
    assert_with_error!(
        env,
        threshold > ltv && threshold <= BPS,
        CollateralError::InvalidLiqThreshold
    );
    assert_with_error!(
        env,
        threshold * (BPS + bonus) <= BPS * BPS,
        CollateralError::InvalidLiqThreshold
    );
}

/// Validates a per-spoke liquidation-curve override: the health factor a
/// liquidated position is restored to, the health factor at/below which the
/// max bonus applies, and the factor scaling the bonus increment between them.
/// `bonus_factor_bps` is capped at `BPS` (100%) because
/// `calculate_linear_bonus_with_target` adds the scaled increment to the base
/// bonus without re-clamping to the dynamic max; a factor above 100% could
/// push the realized bonus past the seizure-safety ceiling. `target_hf_wad` is
/// bounded above by `MAX_LIQUIDATION_TARGET_HF_WAD` so an oversized (e.g.
/// decimal-scale typo) target cannot overflow `target_hf * total_debt` in the
/// liquidation-target math.
///
/// # Errors
/// * [`CollateralError::InvalidLiquidationCurve`] - `target_hf_wad <= WAD`,
///   `target_hf_wad > MAX_LIQUIDATION_TARGET_HF_WAD`, `hf_for_max_bonus_wad` is
///   outside `(0, target_hf_wad)`, or `bonus_factor_bps > BPS`.
pub fn validate_liquidation_curve(
    env: &Env,
    target_hf_wad: i128,
    hf_for_max_bonus_wad: i128,
    bonus_factor_bps: u32,
) {
    assert_with_error!(
        env,
        target_hf_wad > WAD && target_hf_wad <= MAX_LIQUIDATION_TARGET_HF_WAD,
        CollateralError::InvalidLiquidationCurve
    );
    assert_with_error!(
        env,
        hf_for_max_bonus_wad > 0 && hf_for_max_bonus_wad < target_hf_wad,
        CollateralError::InvalidLiquidationCurve
    );
    assert_with_error!(
        env,
        i128::from(bonus_factor_bps) <= BPS,
        CollateralError::InvalidLiquidationCurve
    );
}

/// Validates a stored oracle price-fluctuation band. The propose path builds the
/// band from a tolerance in `[MIN_TOLERANCE, MAX_TOLERANCE]` as
/// `upper = BPS + tolerance`, `lower = BPS^2 / upper`, so a valid band brackets
/// par (`lower <= BPS <= upper`) within that envelope. Re-checking it at the
/// controller setter keeps a direct call from storing a degenerate/inverted band
/// that would revert every read.
///
/// # Errors
/// * [`OracleError::BadLastTolerance`] - the band is outside the envelope or does
///   not bracket par.
pub fn validate_oracle_tolerance(env: &Env, tolerance: &OraclePriceFluctuation) {
    let bps = BPS as u32;
    assert_with_error!(
        env,
        tolerance.upper_ratio_bps >= bps + MIN_TOLERANCE
            && tolerance.upper_ratio_bps <= bps + MAX_TOLERANCE
            && tolerance.lower_ratio_bps > 0
            && tolerance.lower_ratio_bps <= bps,
        OracleError::BadLastTolerance
    );
}

/// Validates market final-price bounds (USD WAD).
///
/// # Errors
/// * [`OracleError::InvalidSanityBounds`] - the bounds violate
///   `0 < min_wad < max_wad <= MAX_REASONABLE_PRICE_WAD`.
pub fn validate_sanity_bounds(env: &Env, min_wad: i128, max_wad: i128) {
    assert_with_error!(
        env,
        min_wad > 0 && max_wad > 0 && min_wad < max_wad && max_wad <= MAX_REASONABLE_PRICE_WAD,
        OracleError::InvalidSanityBounds
    );
}

/// Requires a `Single`-strategy market's sanity band, measured as the
/// midpoint-relative half-width `(max_wad - min_wad) / (max_wad + min_wad)`, to
/// be within `MAX_SINGLE_SOURCE_SANITY_BAND_BPS`. For a band symmetric around a
/// price `p` (`min = p(1-b)`, `max = p(1+b)`) this metric is exactly `b`, the
/// per-bound % deviation from the price, since the band is the only defense a
/// single unchecked oracle source has against a bad price. `PrimaryWithAnchor`
/// markets are exempt: the anchor's tolerance check at read time is the second
/// opinion. Assumes `0 < min_wad < max_wad <= MAX_REASONABLE_PRICE_WAD`
/// (`validate_sanity_bounds` must run first), so `max_wad + min_wad` cannot
/// overflow.
///
/// # Errors
/// * [`OracleError::SanityBandTooWideForSingleSource`] - `strategy == Single`
///   and `(max_wad - min_wad) / (max_wad + min_wad)` exceeds the threshold.
pub fn validate_single_source_sanity_band(
    env: &Env,
    strategy: OracleStrategy,
    min_wad: i128,
    max_wad: i128,
) {
    if strategy != OracleStrategy::Single {
        return;
    }
    // Round up so a band sitting exactly on the ceiling is kept and anything
    // wider is rejected (stricter direction).
    let band_bps = mul_div_ceil(env, max_wad - min_wad, BPS, max_wad + min_wad);
    assert_with_error!(
        env,
        band_bps <= MAX_SINGLE_SOURCE_SANITY_BAND_BPS,
        OracleError::SanityBandTooWideForSingleSource
    );
}

#[cfg(test)]
#[path = "../tests/validation.rs"]
mod tests;
