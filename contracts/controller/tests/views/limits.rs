use super::*;

// `pool_partial_cap` is the analytical seed for the partial-withdraw search.
// The settlement loop can recover a wrong seed through binary search, so the
// exact analytic values are pinned here at the unit level.

fn ctx(
    supplied_tokens: i128,
    borrowed_tokens: i128,
    cash: i128,
    max_utilization: Ray,
) -> MarketLimitCtx {
    MarketLimitCtx {
        supplied: Ray::from_asset(supplied_tokens * 10_000_000, 7),
        borrowed: Ray::from_asset(borrowed_tokens * 10_000_000, 7),
        cash,
        max_utilization,
        supply_index: Ray::ONE,
        decimals: 7,
        borrow_index: Ray::ONE,
    }
}

// With no utilization ceiling the cap is pool cash bounded by the request.
#[test]
fn pool_partial_cap_is_cash_bound_without_utilization_ceiling() {
    let env = Env::default();
    let market = ctx(1_000, 800, 500 * 10_000_000, Ray::ONE);
    assert_eq!(
        market.pool_partial_cap(&env, 1_000 * 10_000_000),
        500 * 10_000_000
    );
}

// A 50 % ceiling on 1000 supplied / 400 borrowed leaves exactly 200 tokens
// of withdrawable headroom (min supplied = 400 / 0.5 = 800).
#[test]
fn pool_partial_cap_respects_utilization_headroom() {
    let env = Env::default();
    let market = ctx(1_000, 400, 1_000 * 10_000_000, Ray::from(RAY / 2));
    assert_eq!(
        market.pool_partial_cap(&env, 1_000 * 10_000_000),
        200 * 10_000_000
    );
}

// Already above the ceiling: no partial withdrawal is possible.
#[test]
fn pool_partial_cap_is_zero_when_pool_sits_above_ceiling() {
    let env = Env::default();
    let market = ctx(1_000, 600, 1_000 * 10_000_000, Ray::from(RAY / 2));
    assert_eq!(market.pool_partial_cap(&env, 1_000 * 10_000_000), 0);
}
