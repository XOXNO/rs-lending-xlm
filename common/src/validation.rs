use crate::constants::{
    BPS, MAX_LIQUIDATION_TARGET_HF_WAD, MAX_REASONABLE_PRICE_WAD, MAX_TOLERANCE,
    MIN_SANITY_BAND_BPS, MIN_TOLERANCE, RAY_DECIMALS, WAD,
};
use crate::errors::{CollateralError, FlashLoanError, GenericError, OracleError};
use crate::math::fp_core::{mul_div_ceil, mul_div_floor, mul_div_half_up};
use crate::oracle::observation::{MAX_SINGLE_SOURCE_SANITY_BAND_BPS, MAX_TWAP_RECORDS};
use crate::types::OracleTolerance;
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Executable, Vec};

pub fn require_positive_amount(env: &Env, amount: i128) {
    assert_with_error!(env, amount > 0, GenericError::AmountMustBePositive);
}

pub fn require_nonneg_amount(env: &Env, amount: i128) {
    assert_with_error!(env, amount >= 0, GenericError::AmountMustBePositive);
}

#[inline]
pub fn expect_invariant<T>(env: &Env, opt: Option<T>) -> T {
    opt.unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}

pub fn require_non_empty_payments<T>(env: &Env, payments: &Vec<T>) {
    assert_with_error!(env, !payments.is_empty(), GenericError::InvalidPayments);
}

/// Largest cap, in asset units, that `Ray::from_asset` can rescale by
/// `10^(RAY_DECIMALS - asset_decimals)` without overflowing `i128`.
///
/// `cap_to_scaled` then divides by the supply index, which grows the value
/// further when the index sits below `RAY` (bad-debt socialisation floors it at
/// `SUPPLY_INDEX_FLOOR_RAW = RAY / 1_000`, a further 1000x). That second step
/// deliberately **saturates** rather than panicking — see `cap_to_scaled` — so
/// only the upscale needs bounding here. Bounding both would force every
/// 7-decimal cap under ~170M tokens, which is tighter than real risk parameters
/// need.
///
/// This is the single source of truth: the validator, the test fixtures, the
/// Certora specs, and the pre-deploy verification script must all derive from
/// it rather than re-deriving the formula.
pub fn max_cap_for_decimals(asset_decimals: u32) -> i128 {
    let exp = RAY_DECIMALS.saturating_sub(asset_decimals);
    let upscale = 10i128
        .checked_pow(exp)
        .expect("10^(RAY_DECIMALS - asset_decimals) fits i128 for asset_decimals <= RAY_DECIMALS");
    i128::MAX / upscale
}

/// Bounds a supply/borrow cap to the range the controller can scale without
/// overflowing. See [`max_cap_for_decimals`].
///
/// Caps are always enforced and carry no "unlimited" sentinel: `0` means the
/// market accepts nothing on that side, and every larger value is a literal
/// ceiling in asset units. `i128::MAX` therefore has no exemption here — it is
/// rejected like any other out-of-domain value.
pub fn require_cap_within_asset_domain(env: &Env, cap: i128, asset_decimals: u32) {
    if RAY_DECIMALS.checked_sub(asset_decimals).is_none() {
        panic_with_error!(env, CollateralError::AssetDecimalsTooHigh);
    }
    assert_with_error!(
        env,
        cap <= max_cap_for_decimals(asset_decimals),
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

pub fn validate_liquidation_fees(env: &Env, fees_bps: u32) {
    assert_with_error!(
        env,
        i128::from(fees_bps) <= BPS,
        CollateralError::InvalidLiqThreshold
    );
}

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

    let expected_lower = mul_div_half_up(env, BPS, BPS, i128::from(tolerance.upper_ratio_bps));
    assert_with_error!(
        env,
        i128::from(tolerance.lower_ratio_bps) == expected_lower,
        OracleError::BadLastTolerance
    );
}

pub fn validate_sanity_bounds(env: &Env, min_wad: i128, max_wad: i128) {
    assert_with_error!(
        env,
        min_wad > 0 && max_wad > 0 && min_wad < max_wad && max_wad <= MAX_REASONABLE_PRICE_WAD,
        OracleError::InvalidSanityBounds
    );

    let half_width_bps = mul_div_floor(env, max_wad - min_wad, BPS, max_wad + min_wad);
    assert_with_error!(
        env,
        half_width_bps >= MIN_SANITY_BAND_BPS,
        OracleError::InvalidSanityBounds
    );
}

pub fn validate_single_source_sanity_band(env: &Env, is_dual: bool, min_wad: i128, max_wad: i128) {
    if is_dual {
        return;
    }

    let band_bps = mul_div_ceil(env, max_wad - min_wad, BPS, max_wad + min_wad);
    assert_with_error!(
        env,
        band_bps <= MAX_SINGLE_SOURCE_SANITY_BAND_BPS,
        OracleError::SanityBandTooWideForSingleSource
    );
}

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
