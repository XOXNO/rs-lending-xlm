use super::*;
use common::constants::RAY;

#[test]
fn debt_close_amount_uses_pool_full_close_ceiling() {
    let env = Env::default();
    let position = DebtPosition {
        scaled_amount: Ray::from(RAY + RAY * 4 / 10),
    };

    assert_eq!(position.scaled_amount.mul(&env, Ray::ONE).to_asset(0), 1);
    assert_eq!(debt_close_amount(&env, &position, Ray::ONE, 0), 2);
}

/// Snapshot for curve tests: 100 USD debt and collateral, a 0.5 collateral-mix
/// proportion, and caller-supplied health factor / weighted collateral.
fn curve_snap(hf_raw: i128, weighted_raw: i128) -> LiquidationSnapshot {
    LiquidationSnapshot {
        total_debt: Wad::from(100 * WAD),
        total_collateral: Wad::from(100 * WAD),
        weighted_coll: Wad::from(weighted_raw),
        proportion_seized: Wad::from(WAD / 2),
        hf: Wad::from(hf_raw),
    }
}

// The default curve reproduces today's exact `2 * gap_wad` bonus scale.
#[test]
fn default_curve_bonus_matches_legacy_scale() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(None);
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(1_020_000_000_000_000_000i128);

    for hf_raw in [
        100_000_000_000_000_000i128,   // 0.10 -> scale capped at 1
        450_000_000_000_000_000i128,   // 0.45
        510_000_000_000_000_000i128,   // 0.51 == target/2 -> scale exactly 1
        900_000_000_000_000_000i128,   // 0.90
        1_010_000_000_000_000_000i128, // 1.01 (just below target)
    ] {
        let hf = Wad::from(hf_raw);
        let got = calculate_linear_bonus_with_target(&env, hf, base, max, &curve, target);

        // Legacy reference: scale = min(2 * (target - hf) / target, 1).
        let gap_wad = (target - hf).div(&env, target);
        let scale = gap_wad.mul(&env, Wad::from(2 * WAD)).min(Wad::ONE);
        let increment = Wad::from((max - base).raw()).mul(&env, scale).raw();
        let want = Bps::from(base.raw() + increment);

        assert_eq!(got.raw(), want.raw(), "hf={hf_raw}");
    }
}

// hf >= target yields the base bonus unchanged.
#[test]
fn bonus_at_or_above_target_is_base() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(None);
    let base = Bps::from(400i128);
    let max = Bps::from(1_000i128);
    let target = Wad::from(1_020_000_000_000_000_000i128);

    let got = calculate_linear_bonus_with_target(&env, target, base, max, &curve, target);
    assert_eq!(got.raw(), base.raw());
}

// A non-default bonus factor scales the increment; 2.0x doubles it exactly.
#[test]
fn bonus_factor_scales_increment() {
    let env = Env::default();
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(1_020_000_000_000_000_000i128);
    let hf = Wad::from(900_000_000_000_000_000i128);

    let default_curve = LiquidationCurve::from_config(None);
    let default_bonus =
        calculate_linear_bonus_with_target(&env, hf, base, max, &default_curve, target);

    let double_factor = SpokeConfig {
        is_deprecated: false,
        liquidation_target_hf_wad: 0,
        hf_for_max_bonus_wad: 0,
        liquidation_bonus_factor_bps: 20_000,
    };
    let curve_2x = LiquidationCurve::from_config(Some(&double_factor));
    let scaled_bonus = calculate_linear_bonus_with_target(&env, hf, base, max, &curve_2x, target);

    let inc_default = default_bonus.raw() - base.raw();
    let inc_scaled = scaled_bonus.raw() - base.raw();
    assert!(inc_default > 0);
    assert_eq!(inc_scaled, inc_default * 2);
}

// A custom target HF changes the estimated close amount vs the 1.02 default.
#[test]
fn custom_target_changes_estimate() {
    let env = Env::default();
    let snap = curve_snap(950_000_000_000_000_000, 95 * WAD); // hf = 0.95, weighted = 95
    let bounds = BonusBounds {
        base: Bps::from(200i128),
        max: Bps::from(1_000i128),
    };

    let default_curve = LiquidationCurve::from_config(None);
    let (ideal_default, _) = estimate_liquidation_amount(&env, &snap, bounds, &default_curve);

    let custom = SpokeConfig {
        is_deprecated: false,
        liquidation_target_hf_wad: 1_300_000_000_000_000_000, // 1.30 target
        hf_for_max_bonus_wad: 0,
        liquidation_bonus_factor_bps: 0,
    };
    let custom_curve = LiquidationCurve::from_config(Some(&custom));
    let (ideal_custom, _) = estimate_liquidation_amount(&env, &snap, bounds, &custom_curve);

    assert!(ideal_default.raw() > 0);
    assert_ne!(ideal_default.raw(), ideal_custom.raw());
}
