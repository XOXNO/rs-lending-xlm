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
fn single_source_band_accepts_within_threshold() {
    let env = Env::default();
    // ±8% symmetric band: (10_800 - 9_200) / (10_800 + 9_200) = 800 bps < 1_000.
    validate_single_source_sanity_band(&env, OracleStrategy::Single, 9_200, 10_800);
}

#[test]
fn single_source_band_accepts_at_exact_threshold() {
    let env = Env::default();
    // ±10% symmetric band: (11_000 - 9_000) / (11_000 + 9_000) = 1_000 bps ==
    // threshold: inclusive boundary.
    validate_single_source_sanity_band(&env, OracleStrategy::Single, 9_000, 11_000);
}

#[test]
#[should_panic]
fn single_source_band_rejects_above_threshold() {
    let env = Env::default();
    // ±11% symmetric band: (11_100 - 8_900) / (11_100 + 8_900) = 1_100 bps >
    // 1_000: too wide for a single source.
    validate_single_source_sanity_band(&env, OracleStrategy::Single, 8_900, 11_100);
}

#[test]
fn single_source_band_exempts_primary_with_anchor() {
    let env = Env::default();
    // Anchor strategy is exempt from the band-width gate regardless of width.
    validate_single_source_sanity_band(&env, OracleStrategy::PrimaryWithAnchor, 1_000, 100_000);
}

#[test]
fn liquidation_curve_accepts_defaults() {
    let env = Env::default();
    // Mirrors DEFAULT_LIQUIDATION_TARGET_HF_WAD/DEFAULT_HF_FOR_MAX_BONUS_WAD/
    // DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS.
    validate_liquidation_curve(
        &env,
        1_020_000_000_000_000_000,
        510_000_000_000_000_000,
        10_000,
    );
}

#[test]
#[should_panic]
fn liquidation_curve_rejects_target_hf_at_one() {
    let env = Env::default();
    validate_liquidation_curve(&env, WAD, WAD / 2, 10_000);
}

#[test]
#[should_panic]
fn liquidation_curve_rejects_target_hf_below_one() {
    let env = Env::default();
    validate_liquidation_curve(&env, WAD - 1, WAD / 2, 10_000);
}

#[test]
fn liquidation_curve_accepts_target_hf_at_ceiling() {
    let env = Env::default();
    // The ceiling itself is inclusive.
    validate_liquidation_curve(&env, MAX_LIQUIDATION_TARGET_HF_WAD, WAD / 2, 10_000);
}

#[test]
#[should_panic]
fn liquidation_curve_rejects_target_hf_above_ceiling() {
    let env = Env::default();
    // An oversized target (e.g. a decimal-scale typo) is rejected before it can
    // overflow `target_hf * total_debt` in the liquidation-target math.
    validate_liquidation_curve(&env, MAX_LIQUIDATION_TARGET_HF_WAD + 1, WAD / 2, 10_000);
}

#[test]
#[should_panic]
fn liquidation_curve_rejects_hf_for_max_bonus_at_or_above_target() {
    let env = Env::default();
    validate_liquidation_curve(&env, WAD + 100, WAD + 100, 10_000);
}

#[test]
#[should_panic]
fn liquidation_curve_rejects_hf_for_max_bonus_zero() {
    let env = Env::default();
    validate_liquidation_curve(&env, WAD + 100, 0, 10_000);
}

#[test]
#[should_panic]
fn liquidation_curve_rejects_hf_for_max_bonus_negative() {
    let env = Env::default();
    validate_liquidation_curve(&env, WAD + 100, -1, 10_000);
}

#[test]
fn liquidation_curve_accepts_bonus_factor_at_bps_ceiling() {
    let env = Env::default();
    validate_liquidation_curve(&env, WAD + 100, WAD / 2, BPS as u32);
}

#[test]
#[should_panic]
fn liquidation_curve_rejects_bonus_factor_above_bps() {
    let env = Env::default();
    validate_liquidation_curve(&env, WAD + 100, WAD / 2, BPS as u32 + 1);
}

#[test]
fn liquidation_curve_accepts_bonus_factor_zero() {
    let env = Env::default();
    // A zero factor is degenerate (bonus never scales past base) but not unsafe.
    validate_liquidation_curve(&env, WAD + 100, WAD / 2, 0);
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

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, Address};

#[contract]
struct WasmReceiver;

#[contractimpl]
impl WasmReceiver {}

#[test]
fn require_positive_accepts_one() {
    let env = Env::default();
    require_positive_amount(&env, 1);
}

#[test]
#[should_panic]
fn require_positive_rejects_zero() {
    let env = Env::default();
    require_positive_amount(&env, 0);
}

#[test]
fn require_nonneg_accepts_zero() {
    let env = Env::default();
    require_nonneg_amount(&env, 0);
}

#[test]
#[should_panic]
fn require_nonneg_rejects_negative() {
    let env = Env::default();
    require_nonneg_amount(&env, -1);
}

#[test]
fn cap_is_enabled_truth_table() {
    assert!(!cap_is_enabled(0));
    assert!(!cap_is_enabled(-1));
    assert!(!cap_is_enabled(i128::MAX));
    assert!(cap_is_enabled(1));
    assert!(cap_is_enabled(1_000_000));
}

#[test]
fn require_wasm_receiver_accepts_contract() {
    let env = Env::default();
    let receiver = env.register(WasmReceiver, ());
    require_wasm_receiver(&env, &receiver);
}

#[test]
#[should_panic]
fn require_wasm_receiver_rejects_account() {
    let env = Env::default();
    let account = Address::generate(&env);
    require_wasm_receiver(&env, &account);
}

#[test]
fn test_validate_liquidation_fees_accepts_full_bps() {
    let env = Env::default();
    validate_liquidation_fees(&env, crate::constants::BPS as u32);
}

#[test]
#[should_panic(expected = "#113")]
fn test_validate_liquidation_fees_rejects_above_bps() {
    let env = Env::default();
    validate_liquidation_fees(&env, crate::constants::BPS as u32 + 1);
}

#[test]
fn test_validate_twap_records_accepts_bounds() {
    let env = Env::default();
    validate_twap_records(&env, 1);
    validate_twap_records(&env, MAX_TWAP_RECORDS);
}

#[test]
#[should_panic(expected = "#219")]
fn test_validate_twap_records_rejects_zero() {
    let env = Env::default();
    validate_twap_records(&env, 0);
}

#[test]
#[should_panic(expected = "#228")]
fn test_validate_twap_records_rejects_above_max() {
    let env = Env::default();
    validate_twap_records(&env, MAX_TWAP_RECORDS + 1);
}
