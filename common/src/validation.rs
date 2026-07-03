//! Cross-contract guard checks shared by the pool and controller.
//!
//! Each guard panics with a stable protocol error so both contracts report
//! identical error codes for the same malformed input.

use crate::constants::{BPS, MAX_REASONABLE_PRICE_WAD, RAY_DECIMALS};
use crate::errors::{CollateralError, FlashLoanError, GenericError, OracleError};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Executable};

/// Rejects a non-positive amount (`amount <= 0`) with `AmountMustBePositive`.
pub fn require_positive_amount(env: &Env, amount: i128) {
    assert_with_error!(env, amount > 0, GenericError::AmountMustBePositive);
}

/// Rejects a negative amount (`amount < 0`) with `AmountMustBePositive`. Zero is
/// accepted for flows where it carries a sentinel meaning (withdraw-all, zero
/// fee, zero rewards).
pub fn require_nonneg_amount(env: &Env, amount: i128) {
    assert_with_error!(env, amount >= 0, GenericError::AmountMustBePositive);
}

/// Caps of zero/negative or `i128::MAX` mean "no cap configured"; the
/// controller's spoke-cap enforcement and `max_*` previews share this rule.
pub fn cap_is_enabled(cap: i128) -> bool {
    cap > 0 && cap != i128::MAX
}

/// Rejects caps that overflow asset-to-RAY scaling.
/// Disabled sentinels (`0` and `i128::MAX`) pass.
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

/// Rejects a flash-loan receiver that is not a deployed Wasm contract.
pub fn require_wasm_receiver(env: &Env, receiver: &Address) {
    assert_with_error!(
        env,
        matches!(receiver.executable(), Some(Executable::Wasm(_))),
        FlashLoanError::InvalidFlashloanReceiver
    );
}

/// Rejects a protocol liquidation fee above 100% (`BPS`).
///
/// The fee is applied to the seized-collateral bonus at liquidation time; an
/// oversized value would make liquidation planning revert for the asset.
pub fn validate_liquidation_fees(env: &Env, fees_bps: u32) {
    assert_with_error!(
        env,
        i128::from(fees_bps) <= BPS,
        CollateralError::InvalidLiqThreshold
    );
}

/// Validates loan-to-value, liquidation-threshold, and liquidation-bonus in bps.
///
/// Governance and controller setters enforce the same bounds:
///   - `liquidation_threshold` > `loan_to_value` and <= 100% (`BPS`).
///   - `threshold * (1 + bonus) <= 100%`; seizure stays within collateral backing.
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
/// Requires `0 < min < max <= MAX_REASONABLE_PRICE_WAD`.
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
