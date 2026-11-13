use super::*;
use soroban_sdk::testutils::Address as _;

fn asset(env: &Env) -> Address {
    Address::generate(env)
}

fn sample_raw_params(env: &Env) -> MarketParamsRaw {
    MarketParamsRaw {
        max_borrow_rate: RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY / 20,
        slope2: RAY / 10,
        slope3: RAY / 2,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1_000,
        asset_id: asset(env),
        asset_decimals: 7,
        is_flashloanable: false,
        flashloan_fee: 0,
    }
}

#[test]
fn test_hub_asset_key_equal_when_hub_and_asset_match() {
    let env = Env::default();
    let a = asset(&env);
    let lhs = HubAssetKey {
        hub_id: 0,
        asset: a.clone(),
    };
    let rhs = HubAssetKey {
        hub_id: 0,
        asset: a,
    };
    assert_eq!(lhs, rhs);
}

#[test]
fn test_hub_asset_key_unequal_when_hub_id_differs() {
    let env = Env::default();
    let a = asset(&env);
    let lhs = HubAssetKey {
        hub_id: 0,
        asset: a.clone(),
    };
    let rhs = HubAssetKey {
        hub_id: 1,
        asset: a,
    };
    assert_ne!(lhs, rhs);
}

#[test]
fn test_market_params_raw_typed_roundtrip() {
    let env = Env::default();
    let raw = sample_raw_params(&env);
    let typed = MarketParams::from(&raw);
    let back = MarketParamsRaw::from(&typed);
    assert_eq!(back.max_borrow_rate, raw.max_borrow_rate);
    assert_eq!(back.base_borrow_rate, raw.base_borrow_rate);
    assert_eq!(back.slope1, raw.slope1);
    assert_eq!(back.slope2, raw.slope2);
    assert_eq!(back.slope3, raw.slope3);
    assert_eq!(back.mid_utilization, raw.mid_utilization);
    assert_eq!(back.optimal_utilization, raw.optimal_utilization);
    assert_eq!(back.max_utilization, raw.max_utilization);
    assert_eq!(back.reserve_factor, raw.reserve_factor);
    assert_eq!(back.asset_id, raw.asset_id);
    assert_eq!(back.asset_decimals, raw.asset_decimals);
    assert_eq!(back.is_flashloanable, raw.is_flashloanable);
    assert_eq!(back.flashloan_fee, raw.flashloan_fee);
}

#[test]
fn test_market_params_rate_model_view_copies_fields() {
    let env = Env::default();
    let raw = sample_raw_params(&env);
    let model = raw.rate_model_view();
    assert_eq!(model.max_borrow_rate, raw.max_borrow_rate);
    assert_eq!(model.base_borrow_rate, raw.base_borrow_rate);
    assert_eq!(model.slope1, raw.slope1);
    assert_eq!(model.slope2, raw.slope2);
    assert_eq!(model.slope3, raw.slope3);
    assert_eq!(model.mid_utilization, raw.mid_utilization);
    assert_eq!(model.optimal_utilization, raw.optimal_utilization);
    assert_eq!(model.max_utilization, raw.max_utilization);
    assert_eq!(model.reserve_factor, raw.reserve_factor);
}

#[test]
fn test_market_params_verify_accepts_valid_config() {
    let env = Env::default();
    sample_raw_params(&env).verify(&env);
}

#[test]
fn test_market_params_verify_accepts_decimals_at_wad() {
    let env = Env::default();
    let mut raw = sample_raw_params(&env);
    raw.asset_decimals = WAD_DECIMALS;
    raw.verify(&env);
}

#[test]
#[should_panic(expected = "#132")]
fn test_market_params_verify_rejects_decimals_above_wad() {
    let env = Env::default();
    let mut raw = sample_raw_params(&env);
    raw.asset_decimals = WAD_DECIMALS + 1;
    raw.verify(&env);
}

#[test]
fn test_market_params_verify_accepts_flashloan_fee_at_cap() {
    let env = Env::default();
    let mut raw = sample_raw_params(&env);
    raw.flashloan_fee = MAX_FLASHLOAN_FEE_BPS as u32;
    raw.verify(&env);
}

#[test]
#[should_panic(expected = "#116")]
fn test_market_params_verify_rejects_flashloan_fee_above_cap() {
    let env = Env::default();
    let mut raw = sample_raw_params(&env);
    raw.flashloan_fee = MAX_FLASHLOAN_FEE_BPS as u32 + 1;
    raw.verify(&env);
}

#[test]
fn test_account_position_raw_typed_roundtrip() {
    let raw = AccountPositionRaw {
        scaled_amount: 12_345 * RAY,
        liquidation_threshold: 8_500,
        liquidation_bonus: 500,
        loan_to_value: 8_000,
        liquidation_fees: 1_000,
    };
    let typed = AccountPosition::from(&raw);
    let back = AccountPositionRaw::from(&typed);
    assert_eq!(back, raw);
}

#[test]
fn test_market_index_raw_typed_roundtrip() {
    let raw = MarketIndexRaw {
        borrow_index: RAY + RAY / 10,
        supply_index: RAY + RAY / 20,
    };
    let typed = MarketIndex::from(&raw);
    let back = MarketIndexRaw::from(&typed);
    assert_eq!(back, raw);
}

#[test]
fn test_pool_state_raw_typed_roundtrip() {
    let raw = PoolStateRaw {
        supplied: 100 * RAY,
        borrowed: 60 * RAY,
        revenue: 5 * RAY,
        borrow_index: RAY,
        supply_index: RAY,
        last_timestamp: 1_700_000_000_000,
        cash: 40_000_000,
    };
    let typed = PoolState::from(&raw);
    let back = PoolStateRaw::from(&typed);
    assert_eq!(back.cash, raw.cash);
    assert_eq!(back.supplied, raw.supplied);
    assert_eq!(back.borrowed, raw.borrowed);
    assert_eq!(back.revenue, raw.revenue);
    assert_eq!(back.borrow_index, raw.borrow_index);
    assert_eq!(back.supply_index, raw.supply_index);
    assert_eq!(back.last_timestamp, raw.last_timestamp);
}

fn valid_rate_model() -> InterestRateModel {
    InterestRateModel {
        base_borrow_rate: RAY / 100,
        slope1: RAY / 10,
        slope2: RAY * 2 / 10,
        slope3: RAY * 3 / 10,
        max_borrow_rate: RAY,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 9 / 10,
        reserve_factor: 1_000,
        is_flashloanable: false,
        flashloan_fee: 0,
    }
}

#[test]
fn test_rate_model_verify_accepts_valid() {
    let env = Env::default();
    valid_rate_model().verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_verify_body_is_not_a_noop() {
    let env = Env::default();
    let mut m = valid_rate_model();
    // slope1 rises 10% over 50% of utilization, so slope2 must rise at least
    // 6% over the next 30%; one raw unit less flattens the curve at mid.
    m.slope2 = m.slope1 * 3 / 5 - 1;
    m.verify(&env);
}

#[test]
fn test_rate_model_full_utilization_rate_at_max_accepted() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.max_borrow_rate = m.base_borrow_rate + m.slope1 + m.slope2 + m.slope3;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_full_utilization_rate_above_max_panics() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.max_borrow_rate = m.base_borrow_rate + m.slope1 + m.slope2 + m.slope3 - 1;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_cap_inside_second_segment_panics() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.base_borrow_rate = RAY / 100;
    m.slope1 = RAY * 4 / 100;
    m.slope2 = RAY * 6 / 100;
    m.slope3 = RAY * 6 / 100;
    m.max_borrow_rate = RAY * 6 / 100;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_monotonic_only_slope1_below_base_panics() {
    let env = Env::default();
    let mut m = valid_rate_model();

    // Steepness 0.2, 1.0, 2.0 and a full-utilization rate of 100%: only
    // `slope1 >= base` fails.
    m.base_borrow_rate = RAY * 2 / 10;
    m.slope1 = RAY / 10;
    m.slope2 = RAY * 3 / 10;
    m.slope3 = RAY * 4 / 10;
    m.max_borrow_rate = RAY;
    m.verify(&env);
}

#[test]
fn test_rate_model_slope2_below_slope1_accepted_when_the_middle_segment_is_steeper() {
    let env = Env::default();
    let mut m = valid_rate_model();

    // slope1 30% over 50% (0.6 per unit), slope2 20% over 30% (0.67),
    // slope3 40% over 20% (2.0): a smaller increment, a steeper segment.
    m.slope1 = RAY * 3 / 10;
    m.slope2 = RAY * 2 / 10;
    m.slope3 = RAY * 4 / 10;
    m.max_borrow_rate = RAY;
    m.verify(&env);
}

#[test]
fn test_rate_model_slope3_below_slope2_accepted_when_the_top_segment_is_steeper() {
    let env = Env::default();
    let mut m = valid_rate_model();

    // slope2 40% over 30% (1.33 per unit), slope3 30% over 20% (1.5).
    m.slope2 = RAY * 4 / 10;
    m.slope3 = RAY * 3 / 10;
    m.max_borrow_rate = RAY;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_monotonic_only_max_below_slope3_panics() {
    let env = Env::default();
    let mut m = valid_rate_model();

    m.slope3 = RAY * 5 / 10;
    m.max_borrow_rate = RAY * 3 / 10;
    m.verify(&env);
}

#[test]
fn test_rate_model_monotonic_slope1_eq_base_does_not_panic() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.slope1 = m.base_borrow_rate;
    m.verify(&env);
}

#[test]
fn test_rate_model_monotonic_slope2_eq_slope1_does_not_panic() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.slope2 = m.slope1;
    m.verify(&env);
}

#[test]
fn test_rate_model_monotonic_slope3_eq_slope2_does_not_panic() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.slope3 = m.slope2;
    m.verify(&env);
}

#[test]
fn test_rate_model_monotonic_max_eq_slope3_does_not_panic() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.base_borrow_rate = 0;
    m.slope1 = 0;
    m.slope2 = 0;
    m.max_borrow_rate = m.slope3;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#117")]
fn test_rate_model_max_util_below_optimal_panics() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.max_utilization = m.optimal_utilization - 1;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#117")]
fn test_rate_model_max_util_above_ray_panics() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.max_utilization = RAY + 1;
    m.verify(&env);
}

#[test]
fn test_rate_model_max_util_eq_optimal_does_not_panic() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.max_utilization = m.optimal_utilization;
    m.verify(&env);
}

#[test]
fn test_rate_model_max_util_eq_ray_does_not_panic() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.max_utilization = RAY;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_market_params_verify_rejects_a_bad_rate_model() {
    let env = Env::default();
    let mut raw = sample_raw_params(&env);
    // slope1 rises 5% over 50%; slope2 below 3% over the next 30% flattens it.
    raw.slope2 = raw.slope1 * 3 / 5 - 1;
    raw.verify(&env);
}

#[test]
fn test_scaled_position_raw_from_typed_positions() {
    let raw_pos = AccountPositionRaw {
        scaled_amount: 7 * RAY,
        liquidation_threshold: 8_000,
        liquidation_bonus: 500,
        loan_to_value: 7_500,
        liquidation_fees: 100,
    };
    let typed_supply = AccountPosition::from(&raw_pos);
    assert_eq!(
        ScaledPositionRaw::from(&typed_supply).scaled_amount,
        7 * RAY
    );

    let typed_debt = DebtPosition {
        scaled_amount: Ray::from(3 * RAY),
    };
    assert_eq!(ScaledPositionRaw::from(&typed_debt).scaled_amount, 3 * RAY);
}

#[test]
fn test_debt_position_conversions_roundtrip() {
    let raw = DebtPositionRaw {
        scaled_amount: 5 * RAY,
    };
    let typed = DebtPosition::from(&raw);
    assert_eq!(typed.scaled_amount.raw(), 5 * RAY);
    assert_eq!(DebtPositionRaw::from(&typed).scaled_amount, 5 * RAY);

    let from_scaled = DebtPosition::from(&ScaledPositionRaw {
        scaled_amount: 9 * RAY,
    });
    assert_eq!(from_scaled.scaled_amount.raw(), 9 * RAY);
}

#[test]
fn test_pool_strategy_mutation_projects_to_position_mutation() {
    let m = PoolStrategyMutation {
        position: ScaledPositionRaw {
            scaled_amount: 11 * RAY,
        },
        market_index: MarketIndexRaw {
            borrow_index: RAY,
            supply_index: RAY,
        },
        actual_amount: 1_000,
        amount_received: 900,
        asset_decimals: 7,
    };
    let projected = PoolPositionMutation::from(&m);
    assert_eq!(projected.position.scaled_amount, 11 * RAY);
    assert_eq!(projected.actual_amount, 1_000);
    assert_eq!(projected.market_index.borrow_index, RAY);
    assert_eq!(projected.asset_decimals, 7);
}
#[test]
#[should_panic(expected = "#128")]
fn test_rate_model_verify_rejects_negative_base_rate() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.base_borrow_rate = -1;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#130")]
fn test_rate_model_verify_rejects_max_equal_to_base() {
    let env = Env::default();
    let mut m = valid_rate_model();

    m.base_borrow_rate = RAY;
    m.slope1 = RAY;
    m.slope2 = RAY;
    m.slope3 = RAY;
    m.max_borrow_rate = RAY;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#131")]
fn test_rate_model_verify_rejects_max_above_cap() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.max_borrow_rate = MAX_BORROW_RATE_RAY + 1;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#117")]
fn test_rate_model_verify_rejects_zero_mid_util() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.mid_utilization = 0;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#117")]
fn test_rate_model_verify_rejects_optimal_equal_to_mid() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.optimal_utilization = m.mid_utilization;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#118")]
fn test_rate_model_verify_rejects_optimal_at_ray() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.optimal_utilization = RAY;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#119")]
fn test_rate_model_verify_rejects_reserve_factor_at_bps() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.reserve_factor = BPS as u32;
    m.verify(&env);
}

/// One percentage point as a RAY rate or utilization.
const PCT: i128 = RAY / 100;

/// A model with the given rates and breakpoints, in percentage points.
fn pct_model(
    base: i128,
    slopes: [i128; 3],
    max: i128,
    mid: i128,
    optimal: i128,
) -> InterestRateModel {
    InterestRateModel {
        base_borrow_rate: base * PCT,
        slope1: slopes[0] * PCT,
        slope2: slopes[1] * PCT,
        slope3: slopes[2] * PCT,
        max_borrow_rate: max * PCT,
        mid_utilization: mid * PCT,
        optimal_utilization: optimal * PCT,
        max_utilization: RAY,
        reserve_factor: 1_000,
        is_flashloanable: false,
        flashloan_fee: 0,
    }
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_verify_rejects_a_curve_that_flattens_at_mid() {
    // Audit I-08 follow-up: slope1 rises 0.5 per unit of utilization and slope2
    // only 0.3, although slope2 > slope1. The increments are ordered; the
    // steepness is not.
    let env = Env::default();
    pct_model(1, [10, 15, 15], 100, 20, 70).verify(&env);
}

#[test]
fn test_rate_model_verify_accepts_a_curve_that_steepens_across_a_narrow_segment() {
    // slope2 < slope1, but over a segment one seventh as wide: steepness
    // 13/70 per unit, then 5/10, then 60/20.
    let env = Env::default();
    pct_model(1, [13, 5, 60], 79, 70, 80).verify(&env);
    pct_model(1, [13, 5, 60], 100, 70, 80).verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_verify_rejects_a_curve_that_flattens_at_optimal() {
    // slope2 rises 0.25 per unit over 40%; slope3 rises 0.2 per unit over 20%.
    let env = Env::default();
    pct_model(0, [2, 10, 4], 20, 40, 80).verify(&env);
}

#[test]
fn test_rate_model_verify_accepts_equal_steepness_at_both_kinks() {
    // A straight line from 0% to 100%: every live mainnet curve has equal
    // steepness at mid, and the zero-rate markets at both kinks.
    let env = Env::default();
    pct_model(0, [2, 4, 4], 10, 20, 60).verify(&env);
    pct_model(0, [0, 0, 0], 1, 49, 99).verify(&env);
    // XLM and USDC on mainnet.
    pct_model(0, [2, 2, 60], 64, 40, 80).verify(&env);
    let mut usdc = pct_model(0, [0, 0, 20], 25, 46, 92);
    usdc.slope1 = 25 * PCT / 10;
    usdc.slope2 = 25 * PCT / 10;
    usdc.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_verify_rejects_one_raw_unit_below_equal_steepness_at_optimal() {
    let env = Env::default();
    let mut m = pct_model(0, [2, 4, 4], 10, 20, 60);
    m.slope3 -= 1;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_verify_rejects_negative_slope2() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.slope2 = -1;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_verify_rejects_negative_slope3() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.slope3 = -1;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_verify_rejects_slope1_below_base() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.slope1 = m.base_borrow_rate - 1;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_verify_rejects_slope2_above_max() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.slope2 = m.max_borrow_rate + 1;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_verify_rejects_a_steep_first_segment_one_unit_wide() {
    // slope1 over a 1-raw-unit segment is a steepness of 10^25 per raw unit;
    // the comparison saturates instead of overflowing and still rejects.
    let env = Env::default();
    let mut m = valid_rate_model();
    m.mid_utilization = 1;
    m.verify(&env);
}

#[test]
fn test_rate_model_verify_accepts_a_top_segment_one_unit_wide() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.optimal_utilization = RAY - 1;
    m.max_utilization = RAY;
    m.verify(&env);
}

/// Exact reference: `rise * next_width <= next_rise * width` in 256 bits.
fn steepness_reference(
    env: &Env,
    rise: i128,
    width: i128,
    next_rise: i128,
    next_width: i128,
) -> bool {
    use soroban_sdk::I256;
    let lhs = I256::from_i128(env, rise).mul(&I256::from_i128(env, next_width));
    let rhs = I256::from_i128(env, next_rise).mul(&I256::from_i128(env, width));
    lhs <= rhs
}

#[test]
fn test_steepness_comparison_is_exact_across_magnitudes() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    let mut seed: u64 = 0x1801_2026_0000_0008;
    let mut draw = |bound: i128| -> i128 {
        let mut word = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            u128::from(seed >> 16)
        };
        let wide = (word() << 96) ^ (word() << 48) ^ word();
        (wide % (bound.max(1) as u128)) as i128
    };
    let mut both = [0u32; 2];
    for i in 0..2_000 {
        // Mix full-range values with near-boundary ones and one-unit widths.
        let width = 1 + if i % 7 == 0 { draw(3) } else { draw(RAY - 1) };
        let next_width = 1 + if i % 5 == 0 { draw(3) } else { draw(RAY - 1) };
        let rise = draw(MAX_BORROW_RATE_RAY + 1);
        let next_rise = if i % 3 == 0 {
            // At or one unit around the exact boundary, when it fits.
            let boundary = Ray::from(rise).mul_ray_ratio_floor_saturating(
                &env,
                Ray::from(next_width),
                Ray::from(width),
            );
            boundary
                .raw()
                .saturating_add(draw(3))
                .saturating_sub(1)
                .clamp(0, MAX_BORROW_RATE_RAY)
        } else {
            draw(MAX_BORROW_RATE_RAY + 1)
        };
        let got = steepness_does_not_fall(
            &env,
            rise,
            Ray::from(width),
            next_rise,
            Ray::from(next_width),
        );
        let want = steepness_reference(&env, rise, width, next_rise, next_width);
        assert_eq!(
            got, want,
            "rise={rise} width={width} next_rise={next_rise} next_width={next_width}"
        );
        both[usize::from(got)] += 1;
    }
    assert!(
        both[0] > 100 && both[1] > 100,
        "both outcomes covered: {both:?}"
    );
}
