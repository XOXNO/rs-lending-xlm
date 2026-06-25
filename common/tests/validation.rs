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

#[test]
fn cap_domain_accepts_disabled_and_reasonable() {
    let env = Env::default();
    // 0 and i128::MAX are disabled sentinels; a real config cap
    // (250_000_000_000_000 at 7 decimals) is well within the from_asset domain.
    require_cap_within_asset_domain(&env, 0, 7);
    require_cap_within_asset_domain(&env, i128::MAX, 7);
    require_cap_within_asset_domain(&env, 250_000_000_000_000, 7);
    // 27-decimal asset: factor is 10^0 = 1, so any non-MAX cap fits.
    require_cap_within_asset_domain(&env, i128::MAX - 1, 27);
}

#[test]
#[should_panic]
fn cap_domain_rejects_overflowing_cap() {
    let env = Env::default();
    // At 7 decimals the ceiling is i128::MAX / 10^20 (~1.7e18); a cap above it
    // would overflow Ray::from_asset's cap * 10^(27-7) rescale.
    require_cap_within_asset_domain(&env, i128::MAX - 1, 7);
}

#[test]
#[should_panic]
fn cap_domain_rejects_decimals_above_ray() {
    let env = Env::default();
    // asset_decimals > RAY_DECIMALS underflows the exponent; the guard fails
    // closed with AssetDecimalsTooHigh rather than panicking on subtraction.
    require_cap_within_asset_domain(&env, 100, RAY_DECIMALS + 1);
}
