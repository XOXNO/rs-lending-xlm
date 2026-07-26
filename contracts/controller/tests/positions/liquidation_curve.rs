use super::*;
use crate::constants::{
    BAD_DEBT_USD_THRESHOLD, DEFAULT_HF_FOR_MAX_BONUS_WAD, DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
    DEFAULT_LIQUIDATION_TARGET_HF_WAD, WAD,
};
use crate::positions::liquidation::curve::{
    calculate_linear_bonus_with_target, calculate_post_liquidation_hf, is_socializable_bad_debt,
};
use common::types::SpokeConfig;

/// Curve values that `add_spoke` stamps at creation.
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

// Pins the literal so a drifted constant cannot hide behind tests that only
// reference the symbol.
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

// The default curve ramps linearly from base to max as HF falls from target
// (1.10) to the knee (0.80), then holds at max below the knee:
//
//     scale = min((target - hf) / (target - knee), 1)
//     bonus = base + (max - base) * scale
//
// Expected bonuses are hand-computed from that formula rather than re-derived
// with the contract's own fixed-point helpers, so a rounding change inside
// `Wad::div`/`Wad::mul` shows up here instead of cancelling out.
#[test]
fn default_curve_bonus_interpolates_base_to_max_across_the_ramp() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);

    // (health factor, expected bonus in bps)
    for (hf_raw, want_bps) in [
        (100_000_000_000_000_000i128, 1_500i128), // 0.10, far below knee -> max
        (450_000_000_000_000_000i128, 1_500),     // 0.45, below knee     -> max
        (DEFAULT_HF_FOR_MAX_BONUS_WAD, 1_500),    // 0.80 == knee         -> max exactly
        (900_000_000_000_000_000i128, 1_167),     // 0.90, scale 2/3      -> 500 + 667
        (1_050_000_000_000_000_000i128, 667),     // 1.05, scale 1/6      -> 500 + 167
    ] {
        let got =
            calculate_linear_bonus_with_target(&env, Wad::from(hf_raw), base, max, &curve, target);
        assert_eq!(got.raw(), want_bps, "hf={hf_raw}");
    }
}

// hf >= target yields the base bonus unchanged.
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

// A non-default bonus factor scales the increment; 2.0x doubles it exactly.
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
    // Guard: a zero baseline increment would make the doubling assertion below
    // pass vacuously (0 == 0 * 2).
    assert!(
        inc_default > 0,
        "picked an HF with no bonus ramp, so the 2x check proves nothing"
    );
    assert_eq!(inc_scaled, inc_default * 2);
}

// Factor above BPS can push realized bonus past `max` (governance caps at BPS).
#[test]
fn bonus_factor_above_bps_can_exceed_max_uncapped() {
    let env = Env::default();
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);
    // At the knee (0.80) scale saturates at 1.
    let hf = Wad::from(DEFAULT_HF_FOR_MAX_BONUS_WAD);

    let over_cap = SpokeConfig {
        liquidation_bonus_factor_bps: 20_000, // 200%, above the enforced BPS ceiling
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

// At factor == BPS, realized bonus never exceeds `max` across the HF range.
#[test]
fn bonus_factor_at_bps_ceiling_never_exceeds_max() {
    let env = Env::default();
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);
    let curve = LiquidationCurve::from_config(&default_spoke_config()); // factor == BPS

    for hf_raw in [
        DEFAULT_LIQUIDATION_TARGET_HF_WAD - WAD / 100, // just below target 1.10
        900_000_000_000_000_000i128,
        700_000_000_000_000_000i128,
        DEFAULT_HF_FOR_MAX_BONUS_WAD, // 0.80 == knee -> scale saturates at 1
        100_000_000_000_000_000i128,  // below knee -> scale still 1
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

// Restoring a higher target HF takes a larger repayment, so raising the spoke's
// target from 1.10 to 1.30 must increase the ideal close amount — not merely
// change it.
#[test]
fn higher_target_hf_raises_the_ideal_close_amount() {
    let env = Env::default();
    // 100 USD debt/collateral, 0.5 mix proportion.
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
        liquidation_target_hf_wad: 1_300_000_000_000_000_000, // 1.30 target
        hf_for_max_bonus_wad: 650_000_000_000_000_000,        // target / 2
        ..default_spoke_config()
    };
    let custom_curve = LiquidationCurve::from_config(&custom);
    let (ideal_custom, _) = estimate_liquidation_amount(&env, &s, bounds, &custom_curve);

    // Guard: a zero baseline would make the comparison below meaningless.
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

// The post-liquidation HF must weight the seized side by 1 + bonus.
#[test]
fn post_liquidation_hf_applies_bonus_on_seized_weight() {
    let env = Env::default();
    // W=100, D=100, p=1, repay 10 at 10% bonus: seized weighted = 11,
    // HF = 89/90.
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

// The effective threshold ceils and the derived max floors: at exactly 50%
// the bound is exactly 100% (10000 bps); any drifted rounding constant moves
// it off this value.
#[test]
fn max_bonus_for_threshold_is_exact_at_half() {
    let env = Env::default();
    assert_eq!(
        max_bonus_for_threshold(&env, Wad::from(WAD / 2)).raw(),
        10_000
    );
}

// The HF-preserving cap returns `None` on each of the two independent
// no-cap conditions (`proportion <= 0` OR `hf >= WAD`) and a finite floored
// cap in the toxic band. The two `None` cases must hold independently: an
// account with seizable collateral but hf >= 1 needs no cap, and a
// zero-proportion account must short-circuit before the `hf/p` division.
#[test]
fn max_hf_preserving_bonus_none_on_each_no_cap_condition() {
    // proportion > 0 but hf >= WAD (healthy): no cap.
    let healthy = snap(50 * WAD, 200 * WAD, 100 * WAD, WAD / 2, 2 * WAD);
    assert_eq!(max_hf_preserving_bonus_bps(&healthy), None);

    // hf < WAD but zero seizable proportion: no cap (also guards the
    // `hf * BPS / proportion` division against a zero divisor).
    let no_seizable = snap(90 * WAD, 100 * WAD, 0, 0, WAD / 2);
    assert_eq!(max_hf_preserving_bonus_bps(&no_seizable), None);

    // Toxic band (proportion 0.45, hf 0.5): finite cap hf/p - 1 = 1111 bps.
    let toxic = snap(90 * WAD, 100 * WAD, 45 * WAD, 45 * WAD / 100, WAD / 2);
    assert_eq!(max_hf_preserving_bonus_bps(&toxic), Some(1_111));
}

// A deeply unhealthy low-threshold position (collateral $100, debt $90,
// threshold 0.45 -> weighted $45, HF 0.5): the curve asks for the max bonus
// (12222 bps) but that seizure rate would ratchet HF on partials, so the
// guard caps the bonus at the largest HF-neutral value, hf/p - 1 = 1111 bps.
// At that bonus the near-full ideal leaves sub-floor dust, so the estimate
// closes the whole debt -- and the $90 * 1.1111 seizure stays inside the
// $100 collateral, leaving no socializable residue.
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

// When even the base bonus would shrink HF (hf/p - 1 below base), partials
// cannot help the account, so the estimate requires a full close at base.
#[test]
fn estimate_full_close_when_base_bonus_ratchets() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    // p = 0.9, HF = 0.9: hf/p - 1 = 0 < base 500.
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

// Outside the toxic band the guard is inert: the HF-scaled bonus applies
// unchanged.
#[test]
fn estimate_safe_region_keeps_scaled_bonus() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    // p = 0.5, HF = 0.95: hf/p - 1 = 9000 bps, scaled bonus is
    // 500 + 9500 * (1.10 - 0.95)/(1.10 - 0.80) = 5250 bps -- under the cap.
    let s = snap(100 * WAD, 200 * WAD, 95 * WAD, WAD / 2, 95 * WAD / 100);
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max: max_bonus_for_threshold(&env, s.proportion_seized),
    };

    let (_d, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(bonus.raw(), 5_250, "scaled bonus kept in the safe region");
}

// The guard invariant, swept: for any estimated (partial) liquidation, a
// repayment at or below the ideal never leaves the account less healthy.
#[test]
fn partial_liquidations_never_reduce_hf() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let collateral = 100 * WAD;
    let mut partials_checked = 0;

    for p_pct in [30i128, 45, 60, 80, 92] {
        for hf_pct in (10..100).step_by(8) {
            let weighted = collateral * p_pct / 100;
            // hf = weighted / debt  =>  debt = weighted / hf.
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
            // A full-close estimate carries no partial to check.
            if ideal.raw() >= s.total_debt.raw() {
                continue;
            }
            for repay in [Wad::from(ideal.raw() / 2), ideal] {
                let post = calculate_post_liquidation_hf(&env, &s, repay, bonus);
                partials_checked += 1;
                assert!(
                    // Half-up rounding in the seizure path may cost 1 ulp.
                    post.raw() + 10 >= s.hf.raw(),
                    "partial at p={p_pct}% hf={hf_pct}% repay={} reduced HF: {} -> {}",
                    repay.raw(),
                    s.hf.raw(),
                    post.raw()
                );
            }
        }
    }

    // The sweep skips full-close estimates, so without this the whole test
    // would pass vacuously if the estimator ever stopped producing partials.
    assert!(
        partials_checked > 0,
        "swept the grid without exercising a single partial liquidation"
    );
}

// The dust guard escalates a sub-floor debt remainder to a full close. A
// high-threshold position (D=$100, C=$104, threshold 0.95 -> weighted $98.8,
// HF 0.988) repays at ~the base bonus but is collateral-capped at
// C/(1+bonus) ≈ $99, leaving < $5 of dust, so the estimate closes it fully.
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

// An above-floor remainder is left untouched: a moderately unhealthy
// low-threshold position (D=$50, C=$100, threshold 0.45 -> weighted $45,
// HF 0.9) repays a partial toward the target and keeps a >$5 debt remainder.
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

// A mildly-unhealthy position restores HF to target with a partial repayment
// bounded by the interpolation, not the collateral cap. Pins the exact ideal
// against an independent reference so a broken denom guard or numerator sign
// (which would return `None` -> the collateral fallback, or invert the sign)
// is caught.
#[test]
fn estimate_target_reachable_returns_interpolated_partial() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    // Zero bonus keeps `1 + bonus == 1`, so `d_max == total_collateral` and the
    // interpolation is the binding constraint (collateral $200 >> repayment).
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

    // Independent reference: with zero bonus the restore-to-target root is
    // `(target*D - W) / (target - p)`, clamped by collateral and total debt.
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

// When collateral already covers the target debt (`target_debt <= weighted`),
// the estimate returns the collateral-capped maximum, not an interpolation
// over a non-positive numerator. At the exact `target_debt == weighted`
// boundary the `<=` admits the collateral-cover branch.
#[test]
fn estimate_collateral_covers_target_returns_collateral_cap() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    // target_hf * D = 1.10 * 100 = 110 == weighted_coll, the branch boundary.
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
    // d_max = collateral / 1 = 120, capped at total debt 100.
    assert_eq!(d.raw(), s.total_debt.raw());
}

// The fallback (target unreachable because the bonus makes the seizure too
// large) closes `collateral / (1 + bonus)`. A flat 50% bonus with a high
// collateral-mix proportion forces `proportion*(1+bonus) >= target`, so the
// closed-form returns `None` and the fallback binds.
#[test]
fn estimate_fallback_divides_collateral_by_one_plus_bonus() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    // base == max pins the bonus at exactly 50%. Under the HF-preservation
    // guard the unreachable-target fallback only fires when the guard is
    // inert (hf >= 1: below 1 the capped bonus keeps the target denominator
    // positive), so pin hf above the target: p*(1+b) = 0.74*1.5 = 1.11 > 1.10
    // makes the target unreachable and the fallback divides the collateral.
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
    // 150 / (1 + 0.5) = 100; a `-` in place of `+` would give 150 / 0.5 = 300,
    // clamped to the $150 debt.
    assert_eq!(d.raw(), 100 * WAD);
}

// The dust guard escalates a sub-floor remainder but leaves an *exactly* $5
// remainder alone (`remaining < $5`, strict). The collateral cap is set so the
// natural ideal leaves precisely `BAD_DEBT_USD_THRESHOLD` of debt; a `<=`
// would wrongly escalate this to a full close.
#[test]
fn estimate_leaves_exactly_five_dollar_remainder_unescalated() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    // Zero bonus, safe region (p = 0.5 <= HF = 0.53): the target-HF root is
    // exactly (1.10*100 - 53) / (1.10 - 0.5) = $95, so remaining is exactly
    // $100 - $95 = $5.
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

// The bonus is monotone in health factor: a lower HF never yields a smaller
// bonus.
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

// The bonus stays within `[base, max]` across the whole HF range.
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
