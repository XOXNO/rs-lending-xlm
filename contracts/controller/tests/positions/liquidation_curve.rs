use super::*;
use crate::constants::{
    BAD_DEBT_USD_THRESHOLD, DEFAULT_HF_FOR_MAX_BONUS_WAD, DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
    DEFAULT_LIQUIDATION_TARGET_HF_WAD, WAD,
};
use crate::positions::liquidation::curve::{
    calculate_linear_bonus_with_target, calculate_post_liquidation_hf, is_socializable_bad_debt,
};
use common::types::SpokeConfig;

fn default_spoke_config() -> SpokeConfig {
    SpokeConfig {
        is_deprecated: false,
        liquidation_target_hf_wad: DEFAULT_LIQUIDATION_TARGET_HF_WAD,
        hf_for_max_bonus_wad: DEFAULT_HF_FOR_MAX_BONUS_WAD,
        liquidation_bonus_factor_bps: DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
    }
}

fn snap(
    debt: i128,
    collateral: i128,
    weighted: i128,
    proportion: i128,
    hf: i128,
) -> LiquidationSnapshot {
    LiquidationSnapshot {
        total_debt: Wad::from(debt),
        total_collateral: Wad::from(collateral),
        weighted_coll: Wad::from(weighted),
        proportion_seized: Wad::from(proportion),
        hf: Wad::from(hf),
    }
}

#[test]
fn bad_debt_threshold_is_five_usd_wad() {
    assert_eq!(BAD_DEBT_USD_THRESHOLD, 5_000_000_000_000_000_000);
}

#[test]
fn bad_debt_socialization_requires_debt_exceeding_collateral_under_threshold() {
    let env = Env::default();
    let collateral = Wad::from(BAD_DEBT_USD_THRESHOLD);
    assert!(is_socializable_bad_debt(
        collateral.checked_add(&env, Wad::from(1)),
        collateral
    ));
    assert!(!is_socializable_bad_debt(collateral, collateral));
    assert!(!is_socializable_bad_debt(
        Wad::from(BAD_DEBT_USD_THRESHOLD + 2 * WAD),
        Wad::from(BAD_DEBT_USD_THRESHOLD + WAD)
    ));
}

#[test]
fn default_curve_bonus_interpolates_base_to_max_across_the_ramp() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);

    for (hf_raw, want_bps) in [
        (100_000_000_000_000_000i128, 1_500i128),
        (450_000_000_000_000_000i128, 1_500),
        (DEFAULT_HF_FOR_MAX_BONUS_WAD, 1_500),
        (900_000_000_000_000_000i128, 1_167),
        (1_050_000_000_000_000_000i128, 667),
    ] {
        let got =
            calculate_linear_bonus_with_target(&env, Wad::from(hf_raw), base, max, &curve, target);
        assert_eq!(got.raw(), want_bps, "hf={hf_raw}");
    }
}

#[test]
fn bonus_at_or_above_target_is_base() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let base = Bps::from(400i128);
    let max = Bps::from(1_000i128);
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);

    let got = calculate_linear_bonus_with_target(&env, target, base, max, &curve, target);
    assert_eq!(got.raw(), base.raw());
}

#[test]
fn bonus_factor_scales_increment() {
    let env = Env::default();
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);
    let hf = Wad::from(900_000_000_000_000_000i128);

    let default_curve = LiquidationCurve::from_config(&default_spoke_config());
    let default_bonus =
        calculate_linear_bonus_with_target(&env, hf, base, max, &default_curve, target);

    let double_factor = SpokeConfig {
        liquidation_bonus_factor_bps: 20_000,
        ..default_spoke_config()
    };
    let curve_2x = LiquidationCurve::from_config(&double_factor);
    let scaled_bonus = calculate_linear_bonus_with_target(&env, hf, base, max, &curve_2x, target);

    let inc_default = default_bonus.raw() - base.raw();
    let inc_scaled = scaled_bonus.raw() - base.raw();

    assert!(
        inc_default > 0,
        "picked an HF with no bonus ramp, so the 2x check proves nothing"
    );
    assert_eq!(inc_scaled, inc_default * 2);
}

#[test]
fn bonus_factor_above_bps_can_exceed_max_uncapped() {
    let env = Env::default();
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);

    let hf = Wad::from(DEFAULT_HF_FOR_MAX_BONUS_WAD);

    let over_cap = SpokeConfig {
        liquidation_bonus_factor_bps: 20_000,
        ..default_spoke_config()
    };
    let curve = LiquidationCurve::from_config(&over_cap);
    let got = calculate_linear_bonus_with_target(&env, hf, base, max, &curve, target);

    assert!(
        got.raw() > max.raw(),
        "expected an over-cap factor to breach max, got {} vs max {}",
        got.raw(),
        max.raw()
    );
}

#[test]
fn bonus_factor_at_bps_ceiling_never_exceeds_max() {
    let env = Env::default();
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    for hf_raw in [
        DEFAULT_LIQUIDATION_TARGET_HF_WAD - WAD / 100,
        900_000_000_000_000_000i128,
        700_000_000_000_000_000i128,
        DEFAULT_HF_FOR_MAX_BONUS_WAD,
        100_000_000_000_000_000i128,
    ] {
        let hf = Wad::from(hf_raw);
        let got = calculate_linear_bonus_with_target(&env, hf, base, max, &curve, target);
        assert!(
            got.raw() <= max.raw(),
            "hf={hf_raw} produced bonus {} exceeding max {}",
            got.raw(),
            max.raw()
        );
    }
}

#[test]
fn higher_target_hf_raises_the_ideal_close_amount() {
    let env = Env::default();

    let s = snap(
        100 * WAD,
        100 * WAD,
        95 * WAD,
        WAD / 2,
        950_000_000_000_000_000,
    );
    let bounds = BonusBounds {
        base: Bps::from(200i128),
        max: Bps::from(1_000i128),
    };

    let default_curve = LiquidationCurve::from_config(&default_spoke_config());
    let (ideal_default, _) = estimate_liquidation_amount(&env, &s, bounds, &default_curve);

    let custom = SpokeConfig {
        liquidation_target_hf_wad: 1_300_000_000_000_000_000,
        hf_for_max_bonus_wad: 650_000_000_000_000_000,
        ..default_spoke_config()
    };
    let custom_curve = LiquidationCurve::from_config(&custom);
    let (ideal_custom, _) = estimate_liquidation_amount(&env, &s, bounds, &custom_curve);

    assert!(
        ideal_default.raw() > 0,
        "default curve closed nothing, so the target comparison proves nothing"
    );
    assert!(
        ideal_custom.raw() > ideal_default.raw(),
        "target 1.30 should close more than target 1.10, got {} vs {}",
        ideal_custom.raw(),
        ideal_default.raw()
    );
}

#[test]
fn post_liquidation_hf_saturates_when_debt_fully_repaid() {
    let env = Env::default();
    let s = snap(
        100 * WAD,
        100 * WAD,
        90 * WAD,
        WAD / 2,
        900_000_000_000_000_000,
    );
    let hf = calculate_post_liquidation_hf(&env, &s, s.total_debt, Bps::from(0i128));
    assert_eq!(hf.raw(), i128::MAX);
}

#[test]
fn post_liquidation_hf_does_not_decrease_for_partial_zero_bonus_repay() {
    let env = Env::default();
    let s = snap(
        100 * WAD,
        100 * WAD,
        90 * WAD,
        WAD / 2,
        900_000_000_000_000_000,
    );
    let hf = calculate_post_liquidation_hf(&env, &s, Wad::from(10 * WAD), Bps::from(0i128));
    assert!(hf >= s.hf);
}

#[test]
fn post_liquidation_hf_applies_bonus_on_seized_weight() {
    let env = Env::default();

    let s = snap(
        100 * WAD,
        120 * WAD,
        100 * WAD,
        WAD,
        900_000_000_000_000_000,
    );
    let hf = calculate_post_liquidation_hf(&env, &s, Wad::from(10 * WAD), Bps::from(1_000i128));
    let expected = Wad::from(89 * WAD).div(&env, Wad::from(90 * WAD));
    assert_eq!(hf.raw(), expected.raw());
}

#[test]
fn max_bonus_for_threshold_is_exact_at_half() {
    let env = Env::default();
    assert_eq!(
        max_bonus_for_threshold(&env, Wad::from(WAD / 2)).raw(),
        10_000
    );
}

#[test]
fn max_hf_preserving_bonus_none_on_each_no_cap_condition() {
    let healthy = snap(50 * WAD, 200 * WAD, 100 * WAD, WAD / 2, 2 * WAD);
    assert_eq!(max_hf_preserving_bonus_bps(&healthy), None);

    let no_seizable = snap(90 * WAD, 100 * WAD, 0, 0, WAD / 2);
    assert_eq!(max_hf_preserving_bonus_bps(&no_seizable), None);

    let toxic = snap(90 * WAD, 100 * WAD, 45 * WAD, 45 * WAD / 100, WAD / 2);
    assert_eq!(max_hf_preserving_bonus_bps(&toxic), Some(1_111));
}

#[test]
fn estimate_toxic_band_caps_bonus_to_hf_neutral() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    let s = snap(90 * WAD, 100 * WAD, 45 * WAD, 45 * WAD / 100, WAD / 2);
    let max = max_bonus_for_threshold(&env, s.proportion_seized);
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max,
    };

    let (d, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(bonus.raw(), 1_111, "bonus capped at hf/p - 1, not the max");
    assert_eq!(d.raw(), s.total_debt.raw(), "dust guard closes the debt");
    let seizure = d.mul(&env, Wad::ONE.checked_add(&env, bonus.to_wad(&env)));
    assert!(
        seizure <= s.total_collateral,
        "capped seizure fits in collateral"
    );
}

#[test]
fn estimate_full_close_when_base_bonus_ratchets() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    let s = snap(
        100 * WAD,
        100 * WAD,
        90 * WAD,
        90 * WAD / 100,
        90 * WAD / 100,
    );
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max: max_bonus_for_threshold(&env, s.proportion_seized),
    };

    let (d, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(bonus.raw(), 500, "full close pays the base bonus");
    assert_eq!(
        d.raw(),
        s.total_debt.raw(),
        "unsafe partials force full close"
    );
}

#[test]
fn estimate_safe_region_keeps_scaled_bonus() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    let s = snap(100 * WAD, 200 * WAD, 95 * WAD, WAD / 2, 95 * WAD / 100);
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max: max_bonus_for_threshold(&env, s.proportion_seized),
    };

    let (_d, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(bonus.raw(), 5_250, "scaled bonus kept in the safe region");
}

#[test]
fn partial_liquidations_never_reduce_hf() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let collateral = 100 * WAD;
    let mut partials_checked = 0;

    for p_pct in [30i128, 45, 60, 80, 92] {
        for hf_pct in (10..100).step_by(8) {
            let weighted = collateral * p_pct / 100;

            let debt = weighted * 100 / hf_pct as i128;
            let s = snap(
                debt,
                collateral,
                weighted,
                p_pct * WAD / 100,
                hf_pct as i128 * WAD / 100,
            );
            let bounds = BonusBounds {
                base: Bps::from(500i128),
                max: max_bonus_for_threshold(&env, s.proportion_seized),
            };

            let (ideal, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);

            if ideal.raw() >= s.total_debt.raw() {
                continue;
            }
            for repay in [Wad::from(ideal.raw() / 2), ideal] {
                let post = calculate_post_liquidation_hf(&env, &s, repay, bonus);
                partials_checked += 1;
                assert!(
                    post.raw() + 10 >= s.hf.raw(),
                    "partial at p={p_pct}% hf={hf_pct}% repay={} reduced HF: {} -> {}",
                    repay.raw(),
                    s.hf.raw(),
                    post.raw()
                );
            }
        }
    }

    assert!(
        partials_checked > 0,
        "swept the grid without exercising a single partial liquidation"
    );
}

#[test]
fn estimate_escalates_sub_floor_debt_dust_to_full_close() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    let s = snap(
        100 * WAD,
        104 * WAD,
        988 * WAD / 10,
        95 * WAD / 100,
        988 * WAD / 1000,
    );
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max: max_bonus_for_threshold(&env, s.proportion_seized),
    };

    let (d, _bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(
        d.raw(),
        s.total_debt.raw(),
        "sub-floor dust escalated to a full close"
    );
}

#[test]
fn estimate_leaves_above_floor_debt_remainder_unescalated() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    let s = snap(
        50 * WAD,
        100 * WAD,
        45 * WAD,
        45 * WAD / 100,
        90 * WAD / 100,
    );
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max: max_bonus_for_threshold(&env, s.proportion_seized),
    };

    let (d, _bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert!(d < s.total_debt, "partial repayment");
    assert!(
        s.total_debt.checked_sub(&env, d) >= Wad::from(BAD_DEBT_USD_THRESHOLD),
        "remainder stays above the socialization floor"
    );
}

#[test]
fn estimate_target_reachable_returns_interpolated_partial() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    let s = snap(
        100 * WAD,
        200 * WAD,
        95 * WAD,
        475 * WAD / 1000,
        95 * WAD / 100,
    );
    let bounds = BonusBounds {
        base: Bps::from(0i128),
        max: Bps::from(0i128),
    };

    let (d, _bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);

    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);
    let target_debt = target.mul(&env, s.total_debt);
    let numerator = target_debt.checked_sub(&env, s.weighted_coll);
    let denominator = target.checked_sub(&env, s.proportion_seized);
    let expected = numerator
        .div(&env, denominator)
        .min(s.total_collateral)
        .min(s.total_debt);

    assert!(
        d < s.total_debt,
        "target-reachable partial, not a full close"
    );
    assert_eq!(d.raw(), expected.raw());
}

#[test]
fn estimate_collateral_covers_target_returns_collateral_cap() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    let s = snap(
        100 * WAD,
        120 * WAD,
        110 * WAD,
        85 * WAD / 100,
        102 * WAD / 100,
    );
    let bounds = BonusBounds {
        base: Bps::from(0i128),
        max: Bps::from(0i128),
    };

    let (d, _bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);

    assert_eq!(d.raw(), s.total_debt.raw());
}

#[test]
fn estimate_fallback_divides_collateral_by_one_plus_bonus() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    let s = snap(
        150 * WAD,
        150 * WAD,
        50 * WAD,
        74 * WAD / 100,
        12 * WAD / 10,
    );
    let bounds = BonusBounds {
        base: Bps::from(5_000i128),
        max: Bps::from(5_000i128),
    };

    let (d, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(bonus.raw(), 5_000);

    assert_eq!(d.raw(), 100 * WAD);
}

#[test]
fn estimate_leaves_exactly_five_dollar_remainder_unescalated() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    let s = snap(100 * WAD, 106 * WAD, 53 * WAD, WAD / 2, 53 * WAD / 100);
    let bounds = BonusBounds {
        base: Bps::from(0i128),
        max: Bps::from(0i128),
    };

    let (d, _bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(
        d.raw(),
        s.total_debt.raw() - BAD_DEBT_USD_THRESHOLD,
        "an exactly-$5 remainder is left as a partial, not escalated"
    );
}

#[test]
fn bonus_monotone_decreasing_in_hf() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let base = Bps::from(500i128);
    let max = Bps::from(2_500i128);
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);

    let mut prev = i128::MAX;
    for pct in (10..=102).step_by(2) {
        let hf = Wad::from(WAD * pct / 100);
        let b = calculate_linear_bonus_with_target(&env, hf, base, max, &curve, target).raw();
        assert!(
            b <= prev,
            "bonus must not increase as HF rises: hf={pct}% bonus={b} prev={prev}"
        );
        prev = b;
    }
}

#[test]
fn bonus_within_base_and_max_bounds() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let base = Bps::from(500i128);
    let max = Bps::from(2_500i128);
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);

    for pct in (5..=110).step_by(3) {
        let hf = Wad::from(WAD * pct / 100);
        let b = calculate_linear_bonus_with_target(&env, hf, base, max, &curve, target).raw();
        assert!(
            b >= base.raw() && b <= max.raw(),
            "bonus {b} out of [{}, {}] at hf={pct}%",
            base.raw(),
            max.raw()
        );
    }
}
