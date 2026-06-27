use super::*;

#[test]
fn fixed_point_scales() {
    assert_eq!(RAY, 1_000_000_000_000_000_000_000_000_000);
    assert_eq!(WAD, 1_000_000_000_000_000_000);
    assert_eq!(BPS, 10_000);
}

#[test]
fn derived_usd_bounds() {
    // 1e9 * WAD
    assert_eq!(
        MAX_REASONABLE_PRICE_WAD,
        1_000_000_000_000_000_000_000_000_000
    );
    // 5 * WAD
    assert_eq!(
        DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD,
        5_000_000_000_000_000_000
    );
}

#[test]
fn ttl_ledger_counts() {
    // ONE_DAY_LEDGERS (17_280) * day spans.
    assert_eq!(TTL_THRESHOLD_INSTANCE, 518_400); // * 30
    assert_eq!(TTL_BUMP_INSTANCE, 3_110_400); // * 180
    assert_eq!(TTL_THRESHOLD_SHARED, 518_400); // * 30
    assert_eq!(TTL_BUMP_SHARED, 3_110_400); // * 180
    assert_eq!(TTL_THRESHOLD_USER, 518_400); // * 30
    assert_eq!(TTL_BUMP_USER, 2_073_600); // * 120
}

#[test]
fn max_borrow_rate_is_two_ray() {
    // 2 * RAY
    assert_eq!(MAX_BORROW_RATE_RAY, 2_000_000_000_000_000_000_000_000_000);
}
