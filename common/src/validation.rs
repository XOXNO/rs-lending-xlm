//! Cross-contract guard checks shared by the pool and controller.
//!
//! Each guard panics with a stable protocol error so both contracts report
//! identical error codes for the same malformed input.

use crate::constants::{BPS, MAX_REASONABLE_PRICE_WAD};
use crate::errors::{CollateralError, FlashLoanError, GenericError, OracleError};
use soroban_sdk::{assert_with_error, Address, Env, Executable};

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

/// Caps of zero/negative or `i128::MAX` mean "no cap configured"; the pool's
/// enforcement and the controller's `max_*` previews share this rule.
pub fn cap_is_enabled(cap: i128) -> bool {
    cap > 0 && cap != i128::MAX
}

/// Rejects a flash-loan receiver that is not a deployed Wasm contract.
pub fn require_wasm_receiver(env: &Env, receiver: &Address) {
    assert_with_error!(
        env,
        matches!(receiver.executable(), Some(Executable::Wasm(_))),
        FlashLoanError::InvalidFlashloanReceiver
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

/// Validates a market's final-price bounds (USD WAD).
///
/// Governance oracle-config validation and controller activation both require
/// `0 < min < max <= MAX_REASONABLE_PRICE_WAD`. This prevents activation with
/// bounds that would revert on the first risk-increasing or liquidation read.
pub fn validate_sanity_bounds(env: &Env, min_wad: i128, max_wad: i128) {
    assert_with_error!(
        env,
        min_wad > 0 && max_wad > 0 && min_wad < max_wad && max_wad <= MAX_REASONABLE_PRICE_WAD,
        OracleError::InvalidSanityBounds
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn risk_bounds_accepts_valid_triple() {
        let env = Env::default();
        // threshold (80%) > ltv (75%); 8000 * (10000 + 500) = 8.4e7 <= 1e8.
        validate_risk_bounds(&env, 7_500, 8_000, 500);
    }

    #[test]
    #[should_panic]
    fn risk_bounds_rejects_ltv_at_or_above_threshold() {
        let env = Env::default();
        validate_risk_bounds(&env, 8_000, 8_000, 500);
    }

    #[test]
    #[should_panic]
    fn risk_bounds_rejects_threshold_above_bps() {
        let env = Env::default();
        validate_risk_bounds(&env, 5_000, 10_001, 0);
    }

    #[test]
    #[should_panic]
    fn risk_bounds_rejects_bonus_breaching_seizure_ceiling() {
        let env = Env::default();
        // 9500 * (10000 + 600) = 1.007e8 > 1e8: bonus exceeds collateral backing.
        validate_risk_bounds(&env, 5_000, 9_500, 600);
    }

    #[test]
    fn sanity_bounds_accepts_valid_band() {
        let env = Env::default();
        validate_sanity_bounds(&env, 1, MAX_REASONABLE_PRICE_WAD);
    }

    #[test]
    #[should_panic]
    fn sanity_bounds_rejects_unset_max() {
        let env = Env::default();
        validate_sanity_bounds(&env, 1, 0);
    }

    #[test]
    #[should_panic]
    fn sanity_bounds_rejects_min_ge_max() {
        let env = Env::default();
        validate_sanity_bounds(&env, 100, 100);
    }

    #[test]
    #[should_panic]
    fn sanity_bounds_rejects_max_above_cap() {
        let env = Env::default();
        validate_sanity_bounds(&env, 1, MAX_REASONABLE_PRICE_WAD + 1);
    }
}
