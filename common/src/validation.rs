//! Cross-contract guard checks shared by the pool and controller.
//!
//! Each guard panics with a stable protocol error so both contracts report
//! identical error codes for the same malformed input.

use crate::constants::{
    BPS, MAX_LIQUIDATION_TARGET_HF_WAD, MAX_REASONABLE_PRICE_WAD, MAX_TOLERANCE,
    MIN_SANITY_BAND_BPS, MIN_TOLERANCE, RAY_DECIMALS, WAD,
};
use crate::errors::{CollateralError, FlashLoanError, GenericError, OracleError};
use crate::math::fp_core::{mul_div_ceil, mul_div_floor, mul_div_half_up};
use crate::oracle::observation::{MAX_SINGLE_SOURCE_SANITY_BAND_BPS, MAX_TWAP_RECORDS};
use crate::types::OracleTolerance;
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Executable, Vec};

/// Strictly positive amount; zero is rejected.
///
/// # Errors
/// * [`GenericError::AmountMustBePositive`] - `amount <= 0`.
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

/// Unwraps a contract-built value or panics with `InternalError`.
///
/// A missing value here means corrupted storage or a caller logic bug after the
/// checks that were supposed to guarantee it — never ordinary user input.
///
/// # Errors
/// * [`GenericError::InternalError`] - the option was `None`.
#[inline]
pub fn expect_invariant<T>(env: &Env, opt: Option<T>) -> T {
    opt.unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}

/// Rejects an empty payment batch.
///
/// # Errors
/// * [`GenericError::InvalidPayments`] - `payments` is empty.
pub fn require_non_empty_payments<T>(env: &Env, payments: &Vec<T>) {
    assert_with_error!(env, !payments.is_empty(), GenericError::InvalidPayments);
}

/// Rejects a supply/borrow cap that cannot be scaled to RAY without overflowing.
/// `i128::MAX` is the disabled sentinel and always passes.
///
/// # Errors
/// * [`CollateralError::AssetDecimalsTooHigh`] - `asset_decimals > RAY_DECIMALS`.
/// * [`CollateralError::InvalidBorrowParams`] - the cap exceeds the RAY-scalable ceiling.
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

/// Requires `receiver` to be a deployed Wasm contract, so a flash-loan callback
/// cannot be routed to an account or a built-in.
///
/// # Errors
/// * [`FlashLoanError::InvalidFlashloanReceiver`] - `receiver` is not Wasm.
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
/// Lower must be the half-up reciprocal of upper (`BPS² / upper`) so the
/// directed ratio check is invariant to source order. Envelope floor
/// `bps - MAX_TOLERANCE` still applies (reciprocal of `bps + MAX_TOLERANCE`
/// always clears it).
///
/// # Errors
/// * [`OracleError::BadLastTolerance`] - inverted/out-of-envelope/non-reciprocal.
pub fn validate_oracle_tolerance(env: &Env, tolerance: &OracleTolerance) {
    let bps = BPS as u32;
    assert_with_error!(
        env,
        tolerance.upper_ratio_bps >= bps + MIN_TOLERANCE
            && tolerance.upper_ratio_bps <= bps + MAX_TOLERANCE
            && tolerance.lower_ratio_bps >= bps - MAX_TOLERANCE
            && tolerance.lower_ratio_bps <= bps,
        OracleError::BadLastTolerance
    );
    // Order-invariant dual agree: lower = half-up(BPS² / upper), same as governance.
    let expected_lower = mul_div_half_up(env, BPS, BPS, i128::from(tolerance.upper_ratio_bps));
    assert_with_error!(
        env,
        i128::from(tolerance.lower_ratio_bps) == expected_lower,
        OracleError::BadLastTolerance
    );
}

/// Validates an oracle sanity band: positive, ordered, under the reasonable-price
/// ceiling, and wide enough that a normal print cannot revert every hard read.
///
/// # Errors
/// * [`OracleError::InvalidSanityBounds`] - bounds are non-positive, unordered,
///   above `MAX_REASONABLE_PRICE_WAD`, or narrower than `MIN_SANITY_BAND_BPS`.
pub fn validate_sanity_bounds(env: &Env, min_wad: i128, max_wad: i128) {
    assert_with_error!(
        env,
        min_wad > 0 && max_wad > 0 && min_wad < max_wad && max_wad <= MAX_REASONABLE_PRICE_WAD,
        OracleError::InvalidSanityBounds
    );
    // Reject a pinched band (no minimum width otherwise): a band barely wider than
    // the live price reverts on the next real print and bricks every hard read.
    // Floor so a true half-width slightly under MIN_SANITY_BAND_BPS cannot pass
    // via ceil inflation (~1 bps slack).
    let half_width_bps = mul_div_floor(env, max_wad - min_wad, BPS, max_wad + min_wad);
    assert_with_error!(
        env,
        half_width_bps >= MIN_SANITY_BAND_BPS,
        OracleError::InvalidSanityBounds
    );
}

/// Single-source markets: midpoint half-width ≤ `MAX_SINGLE_SOURCE_SANITY_BAND_BPS`.
/// Dual-source markets are exempt. Requires prior `validate_sanity_bounds`.
///
/// # Errors
/// * [`OracleError::SanityBandTooWideForSingleSource`] - band too wide for a
///   lone opinion.
pub fn validate_single_source_sanity_band(env: &Env, is_dual: bool, min_wad: i128, max_wad: i128) {
    if is_dual {
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

/// TWAP record count in `[1, MAX_TWAP_RECORDS]`; shared by the governance
/// input validator and the aggregator read path.
///
/// # Errors
/// * [`OracleError::TwapInsufficientObservations`] - zero records.
/// * [`OracleError::TwapRecordsOutOfRange`] - above `MAX_TWAP_RECORDS`.
pub fn validate_twap_records(env: &Env, records: u32) {
    assert_with_error!(env, records != 0, OracleError::TwapInsufficientObservations);
    assert_with_error!(
        env,
        records <= MAX_TWAP_RECORDS,
        OracleError::TwapRecordsOutOfRange
    );
}

#[cfg(test)]
#[path = "../tests/validation.rs"]
mod tests;
