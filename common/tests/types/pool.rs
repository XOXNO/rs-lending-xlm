use super::*;
use soroban_sdk::testutils::Address as _;

fn asset(env: &Env) -> Address {
    Address::generate(env)
}

fn sample_raw_params(env: &Env) -> MarketParamsRaw {
    MarketParamsRaw {
        max_borrow_rate_ray: RAY,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 20,
        slope2_ray: RAY / 10,
        slope3_ray: RAY / 2,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY * 8 / 10,
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: 1_000,
        asset_id: asset(env),
        asset_decimals: 7,
        supply_cap: 0,
        borrow_cap: 0,
        is_flashloanable: false,
        flashloan_fee_bps: 0,
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
    let rhs = HubAssetKey { hub_id: 0, asset: a };
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
    let rhs = HubAssetKey { hub_id: 1, asset: a };
    assert_ne!(lhs, rhs);
}

#[test]
fn test_market_params_raw_typed_roundtrip() {
    let env = Env::default();
    let raw = sample_raw_params(&env);
    let typed = MarketParams::from(&raw);
    let back = MarketParamsRaw::from(&typed);
    assert_eq!(back.max_borrow_rate_ray, raw.max_borrow_rate_ray);
    assert_eq!(back.base_borrow_rate_ray, raw.base_borrow_rate_ray);
    assert_eq!(back.slope1_ray, raw.slope1_ray);
    assert_eq!(back.slope2_ray, raw.slope2_ray);
    assert_eq!(back.slope3_ray, raw.slope3_ray);
    assert_eq!(back.mid_utilization_ray, raw.mid_utilization_ray);
    assert_eq!(back.optimal_utilization_ray, raw.optimal_utilization_ray);
    assert_eq!(back.max_utilization_ray, raw.max_utilization_ray);
    assert_eq!(back.reserve_factor_bps, raw.reserve_factor_bps);
    assert_eq!(back.asset_id, raw.asset_id);
    assert_eq!(back.asset_decimals, raw.asset_decimals);
}

#[test]
fn test_market_params_rate_model_view_copies_fields() {
    let env = Env::default();
    let raw = sample_raw_params(&env);
    let model = raw.rate_model_view();
    assert_eq!(model.max_borrow_rate_ray, raw.max_borrow_rate_ray);
    assert_eq!(model.base_borrow_rate_ray, raw.base_borrow_rate_ray);
    assert_eq!(model.slope1_ray, raw.slope1_ray);
    assert_eq!(model.slope2_ray, raw.slope2_ray);
    assert_eq!(model.slope3_ray, raw.slope3_ray);
    assert_eq!(model.mid_utilization_ray, raw.mid_utilization_ray);
    assert_eq!(model.optimal_utilization_ray, raw.optimal_utilization_ray);
    assert_eq!(model.max_utilization_ray, raw.max_utilization_ray);
    assert_eq!(model.reserve_factor_bps, raw.reserve_factor_bps);
}

#[test]
fn test_market_params_verify_accepts_valid_config() {
    let env = Env::default();
    sample_raw_params(&env).verify(&env);
}

#[test]
#[should_panic(expected = "#132")]
fn test_market_params_verify_rejects_decimals_above_ray() {
    let env = Env::default();
    let mut raw = sample_raw_params(&env);
    raw.asset_decimals = RAY_DECIMALS + 1;
    raw.verify(&env);
}

#[test]
fn test_account_position_raw_typed_roundtrip() {
    let raw = AccountPositionRaw {
        scaled_amount_ray: 12_345 * RAY,
        liquidation_threshold_bps: 8_500,
        liquidation_bonus_bps: 500,
        loan_to_value_bps: 8_000,
    };
    let typed = AccountPosition::from(&raw);
    let back = AccountPositionRaw::from(&typed);
    assert_eq!(back, raw);
}

#[test]
fn test_market_index_raw_typed_roundtrip() {
    let raw = MarketIndexRaw {
        borrow_index_ray: RAY + RAY / 10,
        supply_index_ray: RAY + RAY / 20,
    };
    let typed = MarketIndex::from(&raw);
    let back = MarketIndexRaw::from(&typed);
    assert_eq!(back, raw);
}

#[test]
fn test_pool_state_raw_typed_roundtrip() {
    let raw = PoolStateRaw {
        supplied_ray: 100 * RAY,
        borrowed_ray: 60 * RAY,
        revenue_ray: 5 * RAY,
        borrow_index_ray: RAY,
        supply_index_ray: RAY,
        last_timestamp: 1_700_000_000_000,
        cash: 40_000_000,
    };
    let typed = PoolState::from(&raw);
    let back = PoolStateRaw::from(&typed);
    assert_eq!(back.cash, raw.cash);
    assert_eq!(back.supplied_ray, raw.supplied_ray);
    assert_eq!(back.borrowed_ray, raw.borrowed_ray);
    assert_eq!(back.revenue_ray, raw.revenue_ray);
    assert_eq!(back.borrow_index_ray, raw.borrow_index_ray);
    assert_eq!(back.supply_index_ray, raw.supply_index_ray);
    assert_eq!(back.last_timestamp, raw.last_timestamp);
}
// InterestRateModel::verify boundary coverage.
//
// Slope-monotonicity and max-utilization guards use plain `if { panic }`
// blocks, so comparison and `||` mutations are observable here. The
// `assert_with_error!` checks (base >= 0, max > base, <= MAX_BORROW_RATE_RAY,
// mid > 0, optimal > mid, optimal < RAY, reserve < BPS) hide their conditions
// in macro arguments and are not targeted here.

fn valid_rate_model() -> InterestRateModel {
    InterestRateModel {
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 10,
        slope2_ray: RAY * 2 / 10,
        slope3_ray: RAY * 3 / 10,
        max_borrow_rate_ray: RAY,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY * 8 / 10,
        max_utilization_ray: RAY * 9 / 10,
        reserve_factor_bps: 1_000,
    }
}

#[test]
fn test_rate_model_verify_accepts_valid() {
    let env = Env::default();
    valid_rate_model().verify(&env);
}

// `replace verify with ()`: invalid input must panic, catching a stubbed body.
#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_verify_body_is_not_a_noop() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.slope2_ray = m.slope1_ray - 1; // slope2 < slope1 → non-monotonic.
    m.verify(&env);
}

// Monotonic chain: `||` short-circuit.
// Each test makes one disjunct true and the rest false: `||` panics,
// while `&&` does not.

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_monotonic_only_slope1_below_base_panics() {
    let env = Env::default();
    let mut m = valid_rate_model();
    // slope1 < base, but keep slope2/slope3/max above their predecessors.
    m.base_borrow_rate_ray = RAY * 2 / 10;
    m.slope1_ray = RAY / 10;
    m.slope2_ray = RAY * 3 / 10;
    m.slope3_ray = RAY * 4 / 10;
    m.max_borrow_rate_ray = RAY * 5 / 10;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_monotonic_only_slope2_below_slope1_panics() {
    let env = Env::default();
    let mut m = valid_rate_model();
    // slope2 < slope1 only.
    m.slope1_ray = RAY * 3 / 10;
    m.slope2_ray = RAY * 2 / 10;
    m.slope3_ray = RAY * 4 / 10;
    m.max_borrow_rate_ray = RAY * 5 / 10;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_monotonic_only_slope3_below_slope2_panics() {
    let env = Env::default();
    let mut m = valid_rate_model();
    // slope3 < slope2 only.
    m.slope2_ray = RAY * 4 / 10;
    m.slope3_ray = RAY * 3 / 10;
    m.max_borrow_rate_ray = RAY * 5 / 10;
    m.verify(&env);
}

#[test]
#[should_panic(expected = "#129")]
fn test_rate_model_monotonic_only_max_below_slope3_panics() {
    let env = Env::default();
    let mut m = valid_rate_model();
    // max < slope3 only, while max still > base (avoids MaxRateBelowBase).
    m.slope3_ray = RAY * 5 / 10;
    m.max_borrow_rate_ray = RAY * 3 / 10;
    m.verify(&env);
}

// Monotonic chain: `<` vs `<=`/`==` at exact equality.
// At `a == b`, `<` is false. `<=` or `==` would panic.

#[test]
fn test_rate_model_monotonic_slope1_eq_base_does_not_panic() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.slope1_ray = m.base_borrow_rate_ray; // slope1 == base.
    m.verify(&env);
}

#[test]
fn test_rate_model_monotonic_slope2_eq_slope1_does_not_panic() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.slope2_ray = m.slope1_ray; // slope2 == slope1.
    m.verify(&env);
}

#[test]
fn test_rate_model_monotonic_slope3_eq_slope2_does_not_panic() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.slope3_ray = m.slope2_ray; // slope3 == slope2.
    m.verify(&env);
}

#[test]
fn test_rate_model_monotonic_max_eq_slope3_does_not_panic() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.max_borrow_rate_ray = m.slope3_ray; // max == slope3.
    m.verify(&env);
}

// Max-utilization guard: `max_util < optimal || max_util > RAY`.

// `||` vs `&&`: only the left disjunct is true.
#[test]
#[should_panic(expected = "#117")]
fn test_rate_model_max_util_below_optimal_panics() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.max_utilization_ray = m.optimal_utilization_ray - 1;
    m.verify(&env);
}

// `||` vs `&&`: only the right disjunct is true.
#[test]
#[should_panic(expected = "#117")]
fn test_rate_model_max_util_above_ray_panics() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.max_utilization_ray = RAY + 1;
    m.verify(&env);
}

// `max_util < optimal`, `<` vs `<=`/`==` at equality: at max_util == optimal,
// `<` is false. Right disjunct is also false (optimal < RAY).
#[test]
fn test_rate_model_max_util_eq_optimal_does_not_panic() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.max_utilization_ray = m.optimal_utilization_ray; // == optimal.
    m.verify(&env);
}

// `max_util > RAY`, `>` vs `>=`/`==` at equality: at max_util == RAY,
// `>` is false and the left disjunct is false.
#[test]
fn test_rate_model_max_util_eq_ray_does_not_panic() {
    let env = Env::default();
    let mut m = valid_rate_model();
    m.max_utilization_ray = RAY; // == RAY (upper edge of valid range).
    m.verify(&env);
}

// `verify_rate_model with ()`: wrapper delegates to `rate_model_view().verify()`.
// Non-monotonic slopes must panic.
#[test]
#[should_panic(expected = "#129")]
fn test_market_params_verify_rate_model_delegates() {
    let env = Env::default();
    let mut raw = sample_raw_params(&env);
    raw.slope2_ray = raw.slope1_ray - 1; // slope2 < slope1.
    raw.verify_rate_model(&env);
}
