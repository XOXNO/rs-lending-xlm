//! Cross-contract guard checks shared by the pool and controller.
//!
//! Each guard panics with a stable protocol error so both contracts report
//! identical error codes for the same malformed input.

use crate::constants::{BPS, MAX_REASONABLE_PRICE_WAD, RAY_DECIMALS};
use crate::errors::{CollateralError, FlashLoanError, GenericError, OracleError};
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

#[cfg(test)]
#[path = "../tests/validation.rs"]
mod tests;
