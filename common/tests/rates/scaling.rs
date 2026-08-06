use super::*;
use crate::constants::RAY;
use soroban_sdk::Env;

#[test]
fn test_scaled_to_original() {
    let env = Env::default();
    let scaled = Ray::from(100 * RAY);
    let index = Ray::from(RAY * 105 / 100);
    let result = scaled_to_original(&env, scaled, index);
    let expected = 105 * RAY;
    assert!((result.raw() - expected).abs() <= 1);
}

#[test]
fn test_resolve_withdrawal_partial_uses_ceil_burn() {
    let env = Env::default();
    let index = Ray::from(RAY + RAY / 3);
    let pos_scaled = Ray::from(10 * RAY);
    let amount = 3;
    let (burn, gross) = resolve_withdrawal(&env, amount, pos_scaled, index, 0);
    let expected_burn = calculate_scaled_supply_ceil(&env, amount, 0, index);
    assert_eq!(burn, expected_burn);
    assert_eq!(gross, amount);
    assert_eq!(burn, Ray::from_asset(amount, 0).div_ceil(&env, index));
}

#[test]
fn test_resolve_withdrawal_full_close_pays_floor() {
    let env = Env::default();
    let index = Ray::ONE;
    let pos_scaled = Ray::from(RAY + RAY * 6 / 10);
    let half_up = unscale_supply(&env, pos_scaled, index, 0);
    let floor = unscale_supply_floor(&env, pos_scaled, index, 0);
    assert_eq!(half_up, 2);
    assert_eq!(floor, 1);
    let (burn, gross) = resolve_withdrawal(&env, half_up, pos_scaled, index, 0);
    assert_eq!(burn, pos_scaled);
    assert_eq!(gross, floor);
}

#[test]
fn test_calculate_scaled_borrow_ceils() {
    let env = Env::default();
    let index = Ray::from(RAY + RAY / 2);
    let amount = 1;
    let ceil = calculate_scaled_borrow(&env, amount, 0, index);
    let half_up = Ray::from_asset(amount, 0).div(&env, index);
    assert!(ceil >= half_up);
    assert_eq!(ceil, Ray::from_asset(amount, 0).div_ceil(&env, index));
}

#[test]
fn test_unscale_borrow_ceil_matches_pool_semantics() {
    let env = Env::default();
    let index = Ray::ONE;
    let scaled = Ray::from(RAY + RAY * 4 / 10);
    assert_eq!(unscale_borrow(&env, scaled, index, 0), 1);
    assert_eq!(unscale_borrow_ceil(&env, scaled, index, 0), 2);
}

/// Pins the controller-view / pool-view amount path: half-up mul then half-up
/// asset rescale. This is the exact expansion of `unscale_supply` /
/// `unscale_borrow` and must stay identical to inline `mul().to_asset()`.
#[test]
fn unscale_supply_and_borrow_match_inline_half_up_mul_to_asset() {
    let env = Env::default();
    let decimals = 7u32;
    let cases = [
        (Ray::from(RAY), Ray::ONE),
        (Ray::from(RAY + RAY / 2), Ray::ONE),
        (Ray::from(RAY + RAY * 4 / 10), Ray::ONE),
        (Ray::from(RAY + RAY * 6 / 10), Ray::ONE),
        (Ray::from(10 * RAY + RAY / 3), Ray::from(RAY + RAY / 7)),
        (Ray::from(RAY * 3 / 2), Ray::from(RAY * 11 / 10)),
    ];

    for (scaled, index) in cases {
        let inline = scaled.mul(&env, index).to_asset(decimals);
        assert_eq!(
            unscale_supply(&env, scaled, index, decimals),
            inline,
            "unscale_supply must equal scaled.mul(index).to_asset (half-up)"
        );
        assert_eq!(
            unscale_borrow(&env, scaled, index, decimals),
            inline,
            "unscale_borrow must equal scaled.mul(index).to_asset (half-up)"
        );
        // When fractional residue is non-zero, floor ≤ half-up ≤ ceil.
        let floor = unscale_supply_floor(&env, scaled, index, decimals);
        let ceil = unscale_borrow_ceil(&env, scaled, index, decimals);
        assert!(floor <= inline && inline <= ceil);
    }
}

#[test]
fn unscale_half_up_exact_half_residue_rounds_away_from_zero() {
    let env = Env::default();
    // decimals=0: to_asset is identity on RAY units after mul with index=ONE.
    // scaled = 1.5 RAY → half-up asset amount = 2.
    let scaled_half = Ray::from(RAY + RAY / 2);
    assert_eq!(unscale_supply(&env, scaled_half, Ray::ONE, 0), 2);
    assert_eq!(unscale_borrow(&env, scaled_half, Ray::ONE, 0), 2);
    assert_eq!(unscale_supply_floor(&env, scaled_half, Ray::ONE, 0), 1);
    assert_eq!(unscale_borrow_ceil(&env, scaled_half, Ray::ONE, 0), 2);

    // Just below half: 1.4 → 1 half-up, 2 ceil.
    let scaled_below = Ray::from(RAY + RAY * 4 / 10);
    assert_eq!(unscale_supply(&env, scaled_below, Ray::ONE, 0), 1);
    assert_eq!(unscale_borrow(&env, scaled_below, Ray::ONE, 0), 1);

    // Just above half: 1.6 → 2 half-up, 1 floor.
    let scaled_above = Ray::from(RAY + RAY * 6 / 10);
    assert_eq!(unscale_supply(&env, scaled_above, Ray::ONE, 0), 2);
    assert_eq!(unscale_borrow(&env, scaled_above, Ray::ONE, 0), 2);
}
