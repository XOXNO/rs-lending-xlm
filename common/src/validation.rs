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

/// Validates a (loan-to-value, liquidation-threshold, liquidation-bonus) triple
/// in bps. Enforced identically by the governance contract and the controller's
/// own e-mode / asset-config setters, so an invalid risk config can never be
/// persisted regardless of which owner calls the setter:
///   - `liquidation_threshold` must sit strictly above `loan_to_value` and at or
///     below 100% (`BPS`) — the load-bearing LTV<threshold borrow buffer.
///   - `threshold * (1 + bonus) <= 100%` — liquidation seizure can never exceed
///     the collateral backing a position, so the bonus can never mint bad debt.
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

/// Validates a market's final-price sanity band (USD WAD). Enforced identically
/// by the governance oracle-config validator and the controller's
/// `set_market_oracle_config` activation path, so a market can never go Active
/// with an unset/invalid band — which would otherwise surface only as a runtime
/// revert on the first risk-increasing / liquidation read. Requires
/// `0 < min < max <= MAX_REASONABLE_PRICE_WAD`.
pub fn validate_sanity_bounds(env: &Env, min_wad: i128, max_wad: i128) {
    assert_with_error!(
        env,
        min_wad > 0
            && max_wad > 0
            && min_wad < max_wad
            && max_wad <= MAX_REASONABLE_PRICE_WAD,
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
        // 9500 * (10000 + 600) = 1.007e8 > 1e8: bonus would seize more than backing.
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
