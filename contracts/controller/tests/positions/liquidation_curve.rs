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

fn grid_snap(collateral: i128, p_pct: i128, hf_pct: i128) -> Option<LiquidationSnapshot> {
    let weighted = collateral * p_pct / 100;
    if hf_pct == 0 {
        return None;
    }
    let debt = weighted * 100 / hf_pct;
    if debt == 0 {
        return None;
    }
    Some(snap(
        debt,
        collateral,
        weighted,
        p_pct * WAD / 100,
        hf_pct * WAD / 100,
    ))
}

#[test]
fn hf_preserving_cap_equals_collateral_over_debt_ratio() {
    let collateral = 100 * WAD;
    let mut checked = 0;

    for p_pct in [30i128, 50, 65, 80, 90, 95] {
        for hf_pct in (10..100).step_by(5) {
            let Some(s) = grid_snap(collateral, p_pct, hf_pct) else {
                continue;
            };
            let Some(cap) = max_hf_preserving_bonus_bps(&s) else {
                continue;
            };

            let want = 10_000i128 * s.total_collateral.raw() / s.total_debt.raw() - 10_000;
            assert!(
                (cap - want).abs() <= 1,
                "cap {cap} != BPS*(V/D-1) {want} at p={p_pct}% hf={hf_pct}%"
            );

            let insolvent = s.total_collateral.raw() < s.total_debt.raw();
            assert_eq!(
                cap < 0,
                insolvent,
                "cap sign disagrees with solvency at p={p_pct}% hf={hf_pct}% (cap={cap})"
            );
            checked += 1;
        }
    }
    assert!(checked > 50, "grid too small: only {checked} points");
}

#[test]
fn max_bonus_for_threshold_matches_closed_form() {
    let env = Env::default();

    for p_pct in [10i128, 25, 40, 50, 75, 80, 90, 99] {
        let p = Wad::from(p_pct * WAD / 100);
        let got = max_bonus_for_threshold(&env, p).raw();
        let eff_thr = p_pct * 100;
        let want = 10_000 * (10_000 - eff_thr) / eff_thr;
        assert_eq!(got, want, "max bonus mismatch at p={p_pct}%");

        assert!(
            (10_000 + got) * eff_thr <= 10_000 * 10_000 + eff_thr,
            "max bonus {got} breaks (1+b)*p <= 1 at p={p_pct}%"
        );
    }
}

#[test]
fn returned_bonus_never_exceeds_the_hf_preserving_cap() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let collateral = 100 * WAD;
    let mut partials = 0;

    for p_pct in [30i128, 50, 65, 80, 90] {
        for hf_pct in (10..100).step_by(5) {
            let Some(s) = grid_snap(collateral, p_pct, hf_pct) else {
                continue;
            };
            let bounds = BonusBounds {
                base: Bps::from(500i128),
                max: max_bonus_for_threshold(&env, s.proportion_seized),
            };
            let (ideal, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);

            if ideal.raw() >= s.total_debt.raw() {
                continue;
            }
            partials += 1;
            let cap = max_hf_preserving_bonus_bps(&s)
                .expect("hf < 1 and p > 0 on this grid, so a cap must exist");
            assert!(
                bonus.raw() <= cap,
                "partial bonus {} exceeds cap {cap} at p={p_pct}% hf={hf_pct}%",
                bonus.raw()
            );
        }
    }
    assert!(partials > 0, "grid never produced a partial liquidation");
}

#[test]
fn full_close_region_is_exactly_where_cap_is_below_base() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let collateral = 100 * WAD;
    let base = Bps::from(500i128);
    let (mut escalated, mut partial) = (0, 0);

    for p_pct in [30i128, 50, 65, 80, 90] {
        for hf_pct in (10..100).step_by(4) {
            let Some(s) = grid_snap(collateral, p_pct, hf_pct) else {
                continue;
            };
            let bounds = BonusBounds {
                base,
                max: max_bonus_for_threshold(&env, s.proportion_seized),
            };
            let (ideal, _) = estimate_liquidation_amount(&env, &s, bounds, &curve);
            let cap = max_hf_preserving_bonus_bps(&s).expect("cap exists on this grid");

            if cap < base.raw() {
                escalated += 1;
                assert!(
                    ideal.raw() >= s.total_debt.raw(),
                    "cap {cap} < base but plan did not escalate at p={p_pct}% hf={hf_pct}%"
                );
            } else {
                partial += 1;
                assert!(
                    ideal.raw() <= s.total_debt.raw(),
                    "ideal exceeded total debt at p={p_pct}% hf={hf_pct}%"
                );
            }
        }
    }
    assert!(
        escalated > 0 && partial > 0,
        "grid must exercise both regions (escalated={escalated}, partial={partial})"
    );
}

#[test]
fn no_accepted_liquidation_reduces_health_factor() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let collateral = 100 * WAD;
    let (mut partials, mut closes, mut insolvent_seen) = (0, 0, 0);

    for p_pct in [30i128, 45, 60, 80, 92] {
        for hf_pct in (10..100).step_by(3) {
            let Some(s) = grid_snap(collateral, p_pct, hf_pct) else {
                continue;
            };
            if s.total_collateral.raw() < s.total_debt.raw() {
                insolvent_seen += 1;
            }
            let bounds = BonusBounds {
                base: Bps::from(500i128),
                max: max_bonus_for_threshold(&env, s.proportion_seized),
            };
            let (ideal, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);

            if ideal.raw() >= s.total_debt.raw() {
                closes += 1;
                let post = calculate_post_liquidation_hf(&env, &s, s.total_debt, bonus);
                assert!(
                    post.raw() >= s.hf.raw(),
                    "full close reduced HF at p={p_pct}% hf={hf_pct}%"
                );
                continue;
            }

            partials += 1;
            for num in [1i128, 2, 3, 4] {
                let repay = Wad::from(ideal.raw() * num / 4);
                if repay.raw() == 0 {
                    continue;
                }
                let post = calculate_post_liquidation_hf(&env, &s, repay, bonus);
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

    assert!(partials > 0, "grid never exercised a partial liquidation");
    assert!(closes > 0, "grid never exercised a full-close escalation");
    assert!(
        insolvent_seen > 0,
        "grid never covered an insolvent account — the regression this guards is unreachable"
    );
}

#[test]
fn partial_plans_size_seizure_within_collateral() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let collateral = 100 * WAD;
    let (mut partials, mut escalations) = (0, 0);

    for p_pct in [30i128, 50, 70, 88] {
        for hf_pct in (10..100).step_by(5) {
            let Some(s) = grid_snap(collateral, p_pct, hf_pct) else {
                continue;
            };
            let bounds = BonusBounds {
                base: Bps::from(500i128),
                max: max_bonus_for_threshold(&env, s.proportion_seized),
            };
            let (ideal, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);

            if ideal.raw() >= s.total_debt.raw() {
                escalations += 1;
                continue;
            }

            partials += 1;
            let seized = ideal.raw() * (10_000 + bonus.raw()) / 10_000;
            assert!(
                seized <= s.total_collateral.raw() + WAD / 1_000,
                "partial seizure {seized} exceeds collateral {} at p={p_pct}% hf={hf_pct}%",
                s.total_collateral.raw()
            );
        }
    }
    assert!(
        partials > 0 && escalations > 0,
        "grid must cover both cases (partials={partials}, escalations={escalations})"
    );
}

#[test]
fn full_close_escalation_causes() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let collateral = 100 * WAD;
    let base = Bps::from(500i128);
    let (mut cap_below_base, mut bonus_clamped, mut other) = (0, 0, 0);

    for p_pct in [30i128, 45, 60, 80, 92] {
        for hf_pct in (10..100).step_by(3) {
            let Some(s) = grid_snap(collateral, p_pct, hf_pct) else {
                continue;
            };
            let bounds = BonusBounds {
                base,
                max: max_bonus_for_threshold(&env, s.proportion_seized),
            };
            let (ideal, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
            if ideal.raw() < s.total_debt.raw() {
                continue;
            }
            let cap = max_hf_preserving_bonus_bps(&s).expect("cap exists on this grid");

            if cap < base.raw() {
                cap_below_base += 1;
                assert_eq!(
                    bonus.raw(),
                    base.raw(),
                    "route 1 must fall back to the base bonus at p={p_pct}% hf={hf_pct}%"
                );
            } else if bonus.raw() == cap {
                bonus_clamped += 1;
                let seized = s.total_debt.raw() * (10_000 + bonus.raw()) / 10_000;
                assert!(
                    (seized - s.total_collateral.raw()).abs() <= s.total_collateral.raw() / 1_000,
                    "route 2 should seize the whole collateral: seized {seized} vs V {} \
                     at p={p_pct}% hf={hf_pct}%",
                    s.total_collateral.raw()
                );
            } else {
                other += 1;
            }
        }
    }

    assert!(
        cap_below_base > 0,
        "grid never hit the cap<base escalation (route 1)"
    );
    assert!(
        bonus_clamped > 0,
        "grid never hit the bonus-clamped wipeout escalation (route 2)"
    );
    let _ = other;
}

#[test]
fn bonus_ramp_matches_closed_form_exactly() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let base = Bps::from(500i128);
    let max = Bps::from(2_500i128);
    let target_raw = DEFAULT_LIQUIDATION_TARGET_HF_WAD;
    let knee_raw = DEFAULT_HF_FOR_MAX_BONUS_WAD;
    let target = Wad::from(target_raw);
    let mut ramp_points = 0;

    for hf_pct in 5..=130i128 {
        let hf_raw = WAD * hf_pct / 100;
        let got =
            calculate_linear_bonus_with_target(&env, Wad::from(hf_raw), base, max, &curve, target)
                .raw();

        let want = if hf_raw >= target_raw {
            base.raw()
        } else if hf_raw <= knee_raw {
            max.raw()
        } else {
            ramp_points += 1;
            let scale_wad = (target_raw - hf_raw) * WAD / (target_raw - knee_raw);
            base.raw() + (max.raw() - base.raw()) * scale_wad / WAD
        };

        assert!(
            (got - want).abs() <= 1,
            "bonus {got} != closed form {want} at hf={hf_pct}%"
        );
    }

    assert!(
        ramp_points > 10,
        "sweep never landed strictly inside the ramp ({ramp_points} points)"
    );
}

#[test]
fn max_bonus_for_threshold_matches_spec_and_rounds_against_the_liquidator() {
    let env = Env::default();

    for p_raw in [
        333_333_333_333_333_333i128,
        666_666_666_666_666_666,
        123_456_789_012_345_678,
        987_654_321_098_765_432,
        500_000_000_000_000_001,
    ] {
        let got = max_bonus_for_threshold(&env, Wad::from(p_raw)).raw();

        let eff_thr = ((p_raw * 10_000 + (WAD - 1)) / WAD).clamp(1, 10_000);
        let want = 10_000 * (10_000 - eff_thr) / eff_thr;
        assert_eq!(got, want, "spec mismatch at p={p_raw} (eff_thr={eff_thr})");

        let exact = (WAD - p_raw) * 10_000 / p_raw;
        assert!(
            got <= exact,
            "max bonus {got} exceeds exact (1-p)/p = {exact} at p={p_raw}"
        );
    }
}

#[test]
fn full_close_on_insolvent_account_would_never_be_profitable() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let collateral = 100 * WAD;
    let mut checked = 0;

    for p_pct in [30i128, 50, 80] {
        for hf_pct in (10..100).step_by(5) {
            let Some(s) = grid_snap(collateral, p_pct, hf_pct) else {
                continue;
            };
            if s.total_collateral.raw() >= s.total_debt.raw() {
                continue;
            }
            let bounds = BonusBounds {
                base: Bps::from(500i128),
                max: max_bonus_for_threshold(&env, s.proportion_seized),
            };
            let (ideal, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
            assert_eq!(
                ideal.raw(),
                s.total_debt.raw(),
                "insolvent account should have its ideal set to the full debt at p={p_pct}% hf={hf_pct}%"
            );

            let recovered = (s.total_debt.raw() * (10_000 + bonus.raw()) / 10_000)
                .min(s.total_collateral.raw());
            assert!(
                recovered < s.total_debt.raw(),
                "full close on an insolvent account should be loss-making, but recovered \
                 {recovered} >= debt {} at p={p_pct}% hf={hf_pct}%",
                s.total_debt.raw()
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "grid never covered an insolvent account");
}

fn apply_liquidation(s: &LiquidationSnapshot, repay: i128, bonus_bps: i128) -> LiquidationSnapshot {
    let (v, d, w) = (
        s.total_collateral.raw(),
        s.total_debt.raw(),
        s.weighted_coll.raw(),
    );
    let seized = (repay * (10_000 + bonus_bps) / 10_000).min(v);
    let p = s.proportion_seized.raw();

    let v2 = v - seized;
    let d2 = d - repay;
    let w2 = w - p * seized / WAD;

    snap(
        d2,
        v2,
        w2,
        if v2 > 0 { w2 * WAD / v2 } else { 0 },
        if d2 > 0 { w2 * WAD / d2 } else { 0 },
    )
}

#[test]
fn hf_neutral_bonus_leaves_health_factor_invariant() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let collateral = 100 * WAD;
    let mut checked = 0;

    for p_pct in [45i128, 60, 80, 92] {
        for hf_pct in (10..100).step_by(3) {
            let Some(s) = grid_snap(collateral, p_pct, hf_pct) else {
                continue;
            };
            let bounds = BonusBounds {
                base: Bps::from(500i128),
                max: max_bonus_for_threshold(&env, s.proportion_seized),
            };
            let (_ideal, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
            let cap = max_hf_preserving_bonus_bps(&s).expect("cap exists on this grid");

            let scaled = calculate_linear_bonus_with_target(
                &env,
                s.hf,
                bounds.base,
                bounds.max,
                &curve,
                Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD),
            );
            if !(scaled.raw() > cap && cap >= bounds.base.raw()) {
                continue;
            }
            checked += 1;
            assert_eq!(
                bonus.raw(),
                cap,
                "the clamped arm must pin the bonus to the neutral rate at p={p_pct}% hf={hf_pct}%"
            );

            for num in [1i128, 2, 3] {
                let repay = Wad::from(s.total_debt.raw() * num / 4);
                if repay.raw() == 0 {
                    continue;
                }
                let post = calculate_post_liquidation_hf(&env, &s, repay, bonus);

                assert!(
                    post.raw() >= s.hf.raw(),
                    "hf fell at the neutral rate: p={p_pct}% hf={hf_pct}% repay={}: {} -> {}",
                    repay.raw(),
                    s.hf.raw(),
                    post.raw()
                );

                let remaining_debt = s.total_debt.raw() - repay.raw();
                if remaining_debt > 0 {
                    let bound =
                        s.proportion_seized.raw() * repay.raw() / (10_000 * remaining_debt) + 1;
                    let drift = post.raw() - s.hf.raw();
                    assert!(
                        drift <= 2 * bound,
                        "hf drift {drift} exceeds the 1-bps quantisation bound {bound} \
                         at p={p_pct}% hf={hf_pct}% repay={}",
                        repay.raw()
                    );
                }
            }
        }
    }
    assert!(checked > 0, "grid never reached the neutral-rate arm");
}

#[test]
fn slicing_at_the_neutral_rate_seizes_the_same_total_as_one_full_close() {
    let env = Env::default();
    let start = snap(
        90 * WAD,
        100 * WAD,
        80 * WAD,
        800_000_000_000_000_000,
        888_888_888_888_888_888,
    );
    let cap = max_hf_preserving_bonus_bps(&start).expect("cap exists");
    assert!(
        cap > 0,
        "fixture must be solvent so the neutral rate is positive"
    );

    let one_shot =
        (start.total_debt.raw() * (10_000 + cap) / 10_000).min(start.total_collateral.raw());

    for slices in [2i128, 3, 6] {
        let mut s = start;
        let mut total_seized = 0i128;
        let per_slice = start.total_debt.raw() / slices;

        for _ in 0..slices {
            let step_cap = max_hf_preserving_bonus_bps(&s).expect("cap exists mid-slice");
            assert!(
                (step_cap - cap).abs() <= 2,
                "neutral rate drifted across slices: {cap} -> {step_cap}"
            );
            let repay = per_slice.min(s.total_debt.raw());
            total_seized += (repay * (10_000 + step_cap) / 10_000).min(s.total_collateral.raw());
            s = apply_liquidation(&s, repay, step_cap);
        }

        let drift = (total_seized - one_shot).abs();
        assert!(
            drift <= one_shot / 100_000,
            "{slices} slices seized {total_seized}, one full close seizes {one_shot} \
             (drift {drift}) — slicing must not be profitable"
        );
    }
    let _ = env;
}

#[test]
fn bonus_above_the_neutral_rate_ratchets_coverage_down() {
    let mut s = snap(
        96 * WAD,
        100 * WAD,
        80 * WAD,
        800_000_000_000_000_000,
        833_333_333_333_333_333,
    );
    let cap = max_hf_preserving_bonus_bps(&s).expect("cap exists");
    let base = 500i128;
    assert!(
        (0..base).contains(&cap),
        "fixture must sit in the 0 <= cap < base band, got {cap}"
    );

    let mut coverage = s.total_collateral.raw() * WAD / s.total_debt.raw();
    for i in 1..=4 {
        s = apply_liquidation(&s, 10 * WAD, base);
        let next = s.total_collateral.raw() * WAD / s.total_debt.raw();
        assert!(
            next < coverage,
            "slice {i} at base bonus should erode coverage: {coverage} -> {next}"
        );
        coverage = next;
    }
}
