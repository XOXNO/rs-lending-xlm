use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, Address, Env};

#[test]
fn risk_bounds_accepts_valid_triple() {
    let env = Env::default();

    validate_risk_bounds(&env, 7_500, 8_000, 500);
}

#[test]
#[should_panic(expected = "#113")]
fn risk_bounds_rejects_ltv_at_or_above_threshold() {
    let env = Env::default();
    validate_risk_bounds(&env, 8_000, 8_000, 500);
}

#[test]
#[should_panic(expected = "#113")]
fn risk_bounds_rejects_threshold_above_bps() {
    let env = Env::default();
    validate_risk_bounds(&env, 5_000, 10_001, 0);
}

#[test]
#[should_panic(expected = "#113")]
fn risk_bounds_rejects_bonus_breaching_seizure_ceiling() {
    let env = Env::default();

    validate_risk_bounds(&env, 5_000, 9_500, 600);
}

#[test]
fn sanity_bounds_accepts_valid_band() {
    let env = Env::default();
    validate_sanity_bounds(&env, 1, MAX_REASONABLE_PRICE_WAD);
}

#[test]
#[should_panic(expected = "#224")]
fn sanity_bounds_rejects_unset_max() {
    let env = Env::default();
    validate_sanity_bounds(&env, 1, 0);
}

#[test]
#[should_panic(expected = "#224")]
fn sanity_bounds_rejects_min_ge_max() {
    let env = Env::default();
    validate_sanity_bounds(&env, 100, 100);
}

#[test]
#[should_panic(expected = "#224")]
fn sanity_bounds_rejects_max_above_cap() {
    let env = Env::default();
    validate_sanity_bounds(&env, 1, MAX_REASONABLE_PRICE_WAD + 1);
}

#[test]
#[should_panic(expected = "#224")]
fn sanity_bounds_rejects_pinched_band() {
    let env = Env::default();

    validate_sanity_bounds(&env, 999_900_000_000_000_000, 1_000_100_000_000_000_000);
}

#[test]
#[should_panic(expected = "#224")]
fn sanity_bounds_rejects_band_under_min_width_with_floor() {
    let env = Env::default();

    validate_sanity_bounds(&env, 10_000, 10_099);
}

#[test]
fn sanity_bounds_accepts_band_at_min_width() {
    let env = Env::default();

    validate_sanity_bounds(&env, 994_000_000_000_000_000, 1_006_000_000_000_000_000);
}

#[test]
fn single_source_band_accepts_within_threshold() {
    let env = Env::default();

    validate_single_source_sanity_band(&env, false, 9_200, 10_800);
}

#[test]
fn single_source_band_accepts_at_exact_threshold() {
    let env = Env::default();

    validate_single_source_sanity_band(&env, false, 9_000, 11_000);
}

#[test]
#[should_panic(expected = "#226")]
fn single_source_band_rejects_above_threshold() {
    let env = Env::default();

    validate_single_source_sanity_band(&env, false, 8_900, 11_100);
}

#[test]
fn single_source_band_exempts_dual_source() {
    let env = Env::default();

    validate_single_source_sanity_band(&env, true, 1_000, 100_000);
}

#[test]
fn oracle_tolerance_accepts_reciprocal_band() {
    let env = Env::default();

    validate_oracle_tolerance(
        &env,
        &OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
    );
}

#[test]
#[should_panic(expected = "#208")]
fn oracle_tolerance_rejects_non_reciprocal_lower() {
    let env = Env::default();

    validate_oracle_tolerance(
        &env,
        &OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_500,
        },
    );
}

#[test]
fn oracle_tolerance_accepts_max_envelope_reciprocal() {
    let env = Env::default();

    validate_oracle_tolerance(
        &env,
        &OracleTolerance {
            upper_ratio_bps: 10_000 + MAX_TOLERANCE,
            lower_ratio_bps: 8_000,
        },
    );
}

#[test]
fn oracle_tolerance_accepts_min_envelope_reciprocal() {
    let env = Env::default();

    let upper = 10_000 + MIN_TOLERANCE;
    let lower = mul_div_half_up(&env, BPS, BPS, i128::from(upper)) as u32;
    validate_oracle_tolerance(
        &env,
        &OracleTolerance {
            upper_ratio_bps: upper,
            lower_ratio_bps: lower,
        },
    );
}

#[test]
fn liquidation_curve_accepts_defaults() {
    let env = Env::default();

    validate_liquidation_curve(
        &env,
        1_020_000_000_000_000_000,
        510_000_000_000_000_000,
        10_000,
    );
}

#[test]
#[should_panic(expected = "#134")]
fn liquidation_curve_rejects_target_hf_at_one() {
    let env = Env::default();
    validate_liquidation_curve(&env, WAD, WAD / 2, 10_000);
}

#[test]
#[should_panic(expected = "#134")]
fn liquidation_curve_rejects_target_hf_below_one() {
    let env = Env::default();
    validate_liquidation_curve(&env, WAD - 1, WAD / 2, 10_000);
}

#[test]
fn liquidation_curve_accepts_target_hf_at_ceiling() {
    let env = Env::default();

    validate_liquidation_curve(&env, MAX_LIQUIDATION_TARGET_HF_WAD, WAD / 2, 10_000);
}

#[test]
#[should_panic(expected = "#134")]
fn liquidation_curve_rejects_target_hf_above_ceiling() {
    let env = Env::default();

    validate_liquidation_curve(&env, MAX_LIQUIDATION_TARGET_HF_WAD + 1, WAD / 2, 10_000);
}

#[test]
#[should_panic(expected = "#134")]
fn liquidation_curve_rejects_hf_for_max_bonus_at_or_above_target() {
    let env = Env::default();
    validate_liquidation_curve(&env, WAD + 100, WAD + 100, 10_000);
}

#[test]
#[should_panic(expected = "#134")]
fn liquidation_curve_rejects_hf_for_max_bonus_zero() {
    let env = Env::default();
    validate_liquidation_curve(&env, WAD + 100, 0, 10_000);
}

#[test]
#[should_panic(expected = "#134")]
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
#[should_panic(expected = "#134")]
fn liquidation_curve_rejects_bonus_factor_above_bps() {
    let env = Env::default();
    validate_liquidation_curve(&env, WAD + 100, WAD / 2, BPS as u32 + 1);
}

#[test]
fn liquidation_curve_accepts_bonus_factor_zero() {
    let env = Env::default();

    validate_liquidation_curve(&env, WAD + 100, WAD / 2, 0);
}

#[test]
fn cap_domain_accepts_zero_and_reasonable() {
    let env = Env::default();

    // `0` is a legal cap value: it closes the market on that side rather than
    // disabling the ceiling. `i128::MAX` is covered by its own rejection test.
    require_cap_within_asset_domain(&env, 0, 7);
    require_cap_within_asset_domain(&env, 250_000_000_000_000, 7);
}

/// The ceiling is inclusive, and it has to hold across the whole listable
/// decimal range (`MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS`, 3..=18, enforced
/// by `governance::validate::asset`) -- a market cannot be listed outside it,
/// so those are the only ends worth pinning.
#[test]
fn cap_domain_accepts_ceiling_across_listable_decimals() {
    let env = Env::default();

    assert_eq!(max_cap_for_decimals(3), 170_141_183_460_469);
    assert_eq!(max_cap_for_decimals(7), 1_701_411_834_604_692_317);
    assert_eq!(
        max_cap_for_decimals(18),
        170_141_183_460_469_231_731_687_303_715
    );

    require_cap_within_asset_domain(&env, max_cap_for_decimals(3), 3);
    require_cap_within_asset_domain(&env, max_cap_for_decimals(7), 7);
    require_cap_within_asset_domain(&env, max_cap_for_decimals(18), 18);
}

#[test]
#[should_panic(expected = "#116")]
fn cap_domain_rejects_overflowing_cap() {
    let env = Env::default();

    require_cap_within_asset_domain(&env, i128::MAX - 1, 7);
}

#[test]
#[should_panic(expected = "#116")]
fn cap_domain_rejects_above_ceiling_at_min_listable_decimals() {
    let env = Env::default();

    require_cap_within_asset_domain(&env, max_cap_for_decimals(3) + 1, 3);
}

#[test]
#[should_panic(expected = "#116")]
fn cap_domain_rejects_above_ceiling_at_max_listable_decimals() {
    let env = Env::default();

    require_cap_within_asset_domain(&env, max_cap_for_decimals(18) + 1, 18);
}

/// `i128::MAX` is no longer an "unlimited" sentinel: it is an ordinary cap
/// value and must lose its exemption from the per-asset domain ceiling.
/// Without this, a stored `i128::MAX` would reach `Ray::from_asset`, which
/// rescales by `10^(RAY_DECIMALS - decimals)` under `overflow-checks`, and
/// panic on every supply/borrow instead of being rejected at config time.
#[test]
#[should_panic(expected = "#116")]
fn cap_domain_rejects_i128_max() {
    let env = Env::default();

    require_cap_within_asset_domain(&env, i128::MAX, 7);
}

#[test]
#[should_panic(expected = "#116")]
fn cap_domain_rejects_i128_max_at_min_listable_decimals() {
    let env = Env::default();

    require_cap_within_asset_domain(&env, i128::MAX, 3);
}

#[test]
#[should_panic(expected = "#116")]
fn cap_domain_rejects_i128_max_at_max_listable_decimals() {
    let env = Env::default();

    require_cap_within_asset_domain(&env, i128::MAX, 18);
}

/// A cap at the ceiling scales without panicking even when the supply index
/// has been floored by bad-debt socialisation — `cap_to_scaled` saturates
/// rather than overflowing. Panicking there would brick supply and borrow for
/// the asset until governance lowered the cap.
#[test]
fn cap_at_domain_ceiling_saturates_under_a_floored_supply_index() {
    use crate::constants::SUPPLY_INDEX_FLOOR_RAW;
    use crate::math::fp::Ray;
    use crate::math::fp_core::mul_div_floor_saturating;

    let env = Env::default();
    let ceiling = max_cap_for_decimals(7);

    require_cap_within_asset_domain(&env, ceiling, 7);

    let scaled = mul_div_floor_saturating(
        &env,
        Ray::from_asset(ceiling, 7).raw(),
        crate::constants::RAY,
        SUPPLY_INDEX_FLOOR_RAW,
    );
    assert_eq!(
        scaled,
        i128::MAX,
        "a ceiling cap at the index floor must saturate, not overflow"
    );
}

#[test]
#[should_panic(expected = "#132")]
fn cap_domain_rejects_decimals_above_ray() {
    let env = Env::default();

    require_cap_within_asset_domain(&env, 100, RAY_DECIMALS + 1);
}

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
#[should_panic(expected = "#14")]
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
#[should_panic(expected = "#14")]
fn require_nonneg_rejects_negative() {
    let env = Env::default();
    require_nonneg_amount(&env, -1);
}

#[test]
fn require_wasm_receiver_accepts_contract() {
    let env = Env::default();
    let receiver = env.register(WasmReceiver, ());
    require_wasm_receiver(&env, &receiver);
}

#[test]
#[should_panic(expected = "#412")]
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

#[test]
fn require_non_empty_payments_accepts_a_populated_batch() {
    let env = Env::default();
    let payments = soroban_sdk::vec![&env, 1i128];
    require_non_empty_payments(&env, &payments);
}

#[test]
#[should_panic(expected = "#16")]
fn require_non_empty_payments_rejects_an_empty_batch() {
    let env = Env::default();
    let payments: soroban_sdk::Vec<i128> = soroban_sdk::Vec::new(&env);
    require_non_empty_payments(&env, &payments);
}
