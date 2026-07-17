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

pub fn require_positive_amount(env: &Env, amount: i128) {
    assert_with_error!(env, amount > 0, GenericError::AmountMustBePositive);
}

/// Non-negative amount; zero allowed as sentinel (withdraw-all, zero fee/rewards).
///
/// # Errors
/// * [`GenericError::AmountMustBePositive`] - `amount < 0`.
pub fn require_nonneg_amount(env: &Env, amount: i128) {
    assert_with_error!(env, amount >= 0, GenericError::AmountMustBePositive);
}

/// Cap enabled when `> 0` and not `i128::MAX` (disabled sentinels).
pub fn cap_is_enabled(cap: i128) -> bool {
    cap > 0 && cap != i128::MAX
}

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

pub fn require_wasm_receiver(env: &Env, receiver: &Address) {
    assert_with_error!(
        env,
        matches!(receiver.executable(), Some(Executable::Wasm(_))),
        FlashLoanError::InvalidFlashloanReceiver
    );
}

/// Protocol liquidation fee ≤ 100% (`BPS`).
///
/// # Errors
/// * [`CollateralError::InvalidLiqThreshold`] - `fees_bps` exceeds `BPS`.
pub fn validate_liquidation_fees(env: &Env, fees_bps: u32) {
    assert_with_error!(
        env,
        i128::from(fees_bps) <= BPS,
        CollateralError::InvalidLiqThreshold
    );
}

/// Risk bounds: `ltv < threshold ≤ BPS` and seizure within collateral backing.
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

/// Spoke liquidation curve: `WAD < target ≤ MAX`, `0 < knee < target`,
/// `bonus_factor_bps ≤ BPS` (factor is not re-clamped at apply).
///
/// # Errors
/// * [`CollateralError::InvalidLiquidationCurve`] - bounds violated.
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

/// Oracle tolerance band brackets par within `[MIN_TOLERANCE, MAX_TOLERANCE]`.
///
/// # Errors
/// * [`OracleError::BadLastTolerance`] - inverted/out-of-envelope band.
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

pub fn validate_sanity_bounds(env: &Env, min_wad: i128, max_wad: i128) {
    assert_with_error!(
        env,
        min_wad > 0 && max_wad > 0 && min_wad < max_wad && max_wad <= MAX_REASONABLE_PRICE_WAD,
        OracleError::InvalidSanityBounds
    );
}

/// `Single` strategy: midpoint half-width ≤ `MAX_SINGLE_SOURCE_SANITY_BAND_BPS`.
/// Anchored markets exempt. Requires prior `validate_sanity_bounds`.
///
/// # Errors
/// * [`OracleError::SanityBandTooWideForSingleSource`] - band too wide for Single.
pub fn validate_single_source_sanity_band(
    env: &Env,
    strategy: OracleStrategy,
    min_wad: i128,
    max_wad: i128,
) {
    if strategy != OracleStrategy::Single {
        return;
    }
    // Ceil so exact ceiling is accepted; anything wider is rejected.
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
