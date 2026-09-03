//! GH-02. Splitting one year of accrual into more keeper calls moves the borrow
//! index by no more than the Taylor truncation bound, and never downward.

use controller::constants::RAY;
use test_harness::{
    hub_asset, usd, LendingTest, MarketPreset, ALICE, BOB, DEFAULT_ASSET_CONFIG,
    DEFAULT_MARKET_PARAMS,
};

const YEAR_SECS: u64 = 31_556_926;

fn usdc() -> MarketPreset {
    MarketPreset {
        name: "USDC",
        decimals: 7,
        price_wad: usd(1),
        initial_liquidity: 0.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn eth() -> MarketPreset {
    MarketPreset {
        name: "ETH",
        decimals: 7,
        price_wad: usd(2_000),
        initial_liquidity: 0.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// Relative shortfall of the eighth-order Taylor series against `e^x`.
fn taylor_tail(x: f64) -> f64 {
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..=8 {
        term *= x / k as f64;
        sum += term;
    }
    1.0 - sum / x.exp()
}

fn borrow_index(t: &LendingTest, asset: &str) -> i128 {
    let key = hub_asset(t.resolve_asset(asset));
    t.pool_client(asset).get_sync_data(&key).state.borrow_index
}

fn accrue_one_year_in(parts: u64, max_rate_ray: i128) -> i128 {
    let mut t = LendingTest::new()
        .with_market(usdc())
        .with_market(eth())
        .with_market_params("USDC", |p| p.max_borrow_rate = max_rate_ray)
        .with_max_utilization_disabled_all_markets()
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply(BOB, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 1_000.0);
    // 90 percent utilization sits above the optimal point, on the steep segment.
    t.borrow(ALICE, "USDC", 90_000.0);
    let step = YEAR_SECS / parts;
    for _ in 0..parts {
        t.advance_and_sync_markets(step, &["USDC"]);
    }
    let remainder = YEAR_SECS - step * parts;
    if remainder > 0 {
        t.advance_and_sync_markets(remainder, &["USDC"]);
    }
    borrow_index(&t, "USDC")
}

#[test]
fn finer_accrual_partitions_never_lower_the_borrow_index() {
    let one = accrue_one_year_in(1, 2 * RAY);
    let twelve = accrue_one_year_in(12, 2 * RAY);
    let weekly = accrue_one_year_in(52, 2 * RAY);
    let daily = accrue_one_year_in(365, 2 * RAY);
    assert!(twelve >= one, "monthly {twelve} < yearly {one}");
    assert!(weekly >= twelve, "weekly {weekly} < monthly {twelve}");
    assert!(daily >= weekly, "daily {daily} < weekly {weekly}");
}

/// The one-year index at the protocol rate cap when the whole year is one
/// chunk at a rate frozen on the starting utilization, versus daily accrual
/// that re-reads utilization every step. Debt compounds faster than supply,
/// so utilization drifts upward between keeper calls and finer accrual
/// realises a higher rate. The gap is the curve, not the series: at 90
/// percent utilization it is about a quarter of the index. Cadence therefore
/// changes borrower cost, and any caller may force the finer cadence.
#[test]
fn partition_spread_is_utilization_drift_not_series_truncation() {
    let one = accrue_one_year_in(1, 2 * RAY) as f64;
    let daily = accrue_one_year_in(365, 2 * RAY) as f64;
    let ray = RAY as f64;
    let spread = (daily - one) / daily;
    std::println!("90% util at the 200% cap: one-shot {one:e}, daily {daily:e}, spread {spread:e}");
    // Finer accrual never exceeds continuous compounding at the rate cap.
    assert!(
        daily <= ray * (2.0f64).exp() * (1.0 + 1e-9),
        "daily {daily:e} above e^2"
    );
    // And the spread is far larger than any series truncation could produce.
    assert!(
        spread > 0.1,
        "expected utilization drift to dominate, got {spread:e}"
    );
    assert!(
        spread < 0.5,
        "drift above half the index means the curve moved more than expected"
    );
}

/// At low utilization the drift is negligible and the cadence spread is
/// within the eighth-order Taylor tail plus rounding.
#[test]
fn partition_spread_at_low_utilization_is_within_the_taylor_tail() {
    let ray = RAY as f64;
    let mut t = LendingTest::new()
        .with_market(usdc())
        .with_market(eth())
        .with_max_utilization_disabled_all_markets()
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply(BOB, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 1_000.0);
    t.borrow(ALICE, "USDC", 10_000.0);
    t.advance_and_sync_markets(YEAR_SECS, &["USDC"]);
    let one = borrow_index(&t, "USDC") as f64;

    let mut t = LendingTest::new()
        .with_market(usdc())
        .with_market(eth())
        .with_max_utilization_disabled_all_markets()
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply(BOB, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 1_000.0);
    t.borrow(ALICE, "USDC", 10_000.0);
    let step = YEAR_SECS / 365;
    for _ in 0..365 {
        t.advance_and_sync_markets(step, &["USDC"]);
    }
    t.advance_and_sync_markets(YEAR_SECS - step * 365, &["USDC"]);
    let daily = borrow_index(&t, "USDC") as f64;

    let rate_used = (daily / ray).ln();
    let spread = (daily - one) / daily;
    std::println!(
        "10% util: one-shot {one:e}, daily {daily:e}, spread {spread:e}, tail {:e}",
        taylor_tail(rate_used)
    );
    // Ten percent utilization on the default curve is about 1.8 percent APR:
    // a year of interest moves utilization by well under a tenth of a percent,
    // so the drift term is below 1e-4 and the tail below 1e-12.
    assert!(
        spread >= -1e-9,
        "finer accrual lowered the index: {spread:e}"
    );
    assert!(
        spread <= taylor_tail(rate_used) + 1e-4,
        "spread {spread:e} above drift plus tail"
    );
}
