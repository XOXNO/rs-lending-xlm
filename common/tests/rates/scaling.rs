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
fn calculate_scaled_cap_floors_and_saturates() {
    let env = Env::default();
    let index = Ray::from(RAY + RAY / 2);
    let cap = 10i128;
    let scaled = calculate_scaled_cap(&env, cap, 0, index);
    let expected = Ray::from_asset(cap, 0).div_floor(&env, index);
    assert_eq!(scaled, expected);

    // Overflow path: ray form fits i128, but × RAY / 1 saturates instead of panicking.
    // 1e11 asset units @ 0 decimals → 1e38 ray (fits); 1e38 * RAY overflows mul_div.
    let large_cap = 100_000_000_000i128;
    let tiny_index = Ray::from(1);
    let saturated = calculate_scaled_cap(&env, large_cap, 0, tiny_index);
    assert_eq!(saturated.raw(), i128::MAX);
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

/// 100 tokens + 0.6 stroops at 7 decimals: floor = 1e9, half-up = 1e9 + 1.
fn seven_dec_supply_with_six_tenth_stroop() -> Ray {
    Ray::from(100 * RAY + 6 * 10i128.pow(19))
}

#[test]
fn resolve_net_settle_closes_both_when_conservative_values_match() {
    let env = Env::default();
    let decimals = 7u32;
    let index = Ray::ONE;
    let supply = seven_dec_supply_with_six_tenth_stroop();
    let debt = Ray::from(100 * RAY);

    assert_eq!(
        unscale_supply_floor(&env, supply, index, decimals),
        1_000_000_000
    );
    assert_eq!(unscale_supply(&env, supply, index, decimals), 1_000_000_001);
    assert_eq!(
        unscale_borrow_ceil(&env, debt, index, decimals),
        1_000_000_000
    );

    // Old withdraw∘repay path: debt_ceil = 1e9 < half-up 1e9+1 → partial
    // supply burn, leftover dust shares, even though payable values match.
    let (burn_s, burn_d, settled) =
        resolve_net_settle(&env, i128::MAX, supply, debt, index, index, decimals);
    assert_eq!(settled, 1_000_000_000);
    assert_eq!(burn_s, supply);
    assert_eq!(burn_d, debt);
}

#[test]
fn resolve_net_settle_does_not_use_half_up_to_close_supply() {
    let env = Env::default();
    let decimals = 7u32;
    let index = Ray::ONE;
    let supply = seven_dec_supply_with_six_tenth_stroop();
    // 1 extra RAY unit ceils to one stroop above the supply floor.
    let debt = Ray::from(100 * RAY + 1);

    assert_eq!(
        unscale_borrow_ceil(&env, debt, index, decimals),
        1_000_000_001
    );

    let (burn_s, burn_d, settled) =
        resolve_net_settle(&env, i128::MAX, supply, debt, index, index, decimals);
    assert_eq!(settled, 1_000_000_000);
    assert_eq!(burn_s, supply, "payable supply is exhausted");
    assert_eq!(
        burn_d,
        calculate_scaled_borrow_floor(&env, settled, decimals, index)
    );
    assert!(burn_d < debt, "one stroop of ceiled debt remains unpaid");
}

#[test]
fn resolve_net_settle_partial_keeps_directed_rounding() {
    let env = Env::default();
    let supply_index = Ray::from(RAY + RAY / 3);
    let borrow_index = Ray::from(RAY + RAY / 2);
    let supply = Ray::from(10 * RAY);
    let debt = Ray::from(10 * RAY);
    let amount = 3;

    let (burn_s, burn_d, settled) =
        resolve_net_settle(&env, amount, supply, debt, supply_index, borrow_index, 0);
    assert_eq!(settled, amount);
    assert_eq!(
        burn_s,
        calculate_scaled_supply_ceil(&env, amount, 0, supply_index)
    );
    assert_eq!(
        burn_d,
        calculate_scaled_borrow_floor(&env, amount, 0, borrow_index)
    );
}

// --- what the supply-index floor buys (docs/reference/numeric-bounds.md §4) --
//
// `SUPPLY_INDEX_FLOOR_RAW` (RAY / 1_000) is the value
// `apply_bad_debt_to_supply_index` clamps to. It keeps the share conversions
// well defined — `calculate_scaled_supply` divides by the index — and bounds
// share inflation to 1_000x, which in turn costs three decades of deposit
// headroom in a fully written-down market.

#[test]
fn supply_index_floor_bounds_share_inflation_to_one_thousand_x() {
    use crate::constants::SUPPLY_INDEX_FLOOR_RAW;

    let env = Env::default();
    let floor = Ray::from(SUPPLY_INDEX_FLOOR_RAW);
    let decimals = 7u32;
    // 1,000 whole tokens at 7 decimals.
    let amount = 10_000_000_000i128;

    let at_one = calculate_scaled_supply(&env, amount, decimals, Ray::ONE);
    let at_floor = calculate_scaled_supply(&env, amount, decimals, floor);

    assert_eq!(at_floor.raw(), at_one.raw() * 1_000);
    // The value the shares represent is unchanged; only the share count moved.
    assert_eq!(unscale_supply(&env, at_floor, floor, decimals), amount);
}

#[test]
fn supply_index_floor_costs_three_decades_of_deposit_headroom() {
    use crate::constants::SUPPLY_INDEX_FLOOR_RAW;
    use crate::validation::max_cap_for_decimals;

    let env = Env::default();
    let floor = Ray::from(SUPPLY_INDEX_FLOOR_RAW);
    let decimals = 7u32;

    // At index 1.0 the ceiling is the balance ceiling itself.
    let ceiling = max_cap_for_decimals(decimals);
    assert!(calculate_scaled_supply(&env, ceiling, decimals, Ray::ONE).raw() > 0);

    // At the floor the scaled form is 1_000x larger, so the largest deposit a
    // written-down market can still take is the ceiling divided by 1_000.
    let at_floor_ceiling = ceiling / 1_000;
    assert!(calculate_scaled_supply(&env, at_floor_ceiling, decimals, floor).raw() > 0);
    assert_eq!(at_floor_ceiling / 10i128.pow(decimals), 170_141_183);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn supply_index_floor_makes_a_ceiling_deposit_overflow_its_scaled_form() {
    use crate::constants::SUPPLY_INDEX_FLOOR_RAW;
    use crate::validation::max_cap_for_decimals;

    let env = Env::default();

    let _ = calculate_scaled_supply(
        &env,
        max_cap_for_decimals(7),
        7,
        Ray::from(SUPPLY_INDEX_FLOOR_RAW),
    );
}

#[test]
fn resolve_net_settle_zero_overlap_is_noop() {
    let env = Env::default();
    let decimals = 7u32;
    let dust = Ray::from(10i128.pow(19));
    assert_eq!(unscale_supply_floor(&env, dust, Ray::ONE, decimals), 0);

    let (burn_s, burn_d, settled) = resolve_net_settle(
        &env,
        i128::MAX,
        dust,
        Ray::from(RAY),
        Ray::ONE,
        Ray::ONE,
        decimals,
    );
    assert_eq!((burn_s, burn_d, settled), (Ray::ZERO, Ray::ZERO, 0));
}
