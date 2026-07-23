use super::{enable_dual_source, setup};
use test_harness::{assert_contract_error, errors, usd, usd_cents, ALICE};

#[test]
fn test_tolerance_at_exact_first_boundary() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Default first tolerance = 200 BPS (2%).
    // The controller's tolerance stores pre-computed ratio bounds:
    //   upper = 10000 + 200 = 10200
    //   lower = 10000^2 / 10200 = 9804
    // Set safe price exactly at 2% deviation: $1.02.
    t.set_safe_price("USDC", usd_cents(102), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // At exactly the first boundary, the price stays within first tolerance
    // and uses the safe price directly (most favorable for the user).
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(
        result.is_ok(),
        "borrow should work at first tolerance boundary"
    );
}

#[test]
fn test_tolerance_just_beyond_first_boundary() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Set safe price at 2.1% deviation (just past first tolerance of 2%).
    // This puts it in the second tolerance zone, where the average price is
    // used.
    t.set_safe_price("USDC", usd_cents(103), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Still succeeds (average price used, within second tolerance).
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(
        result.is_ok(),
        "borrow should work between first and second tolerance"
    );
}

#[test]
fn test_safe_price_below_aggregator_blocks_borrow() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Safe price 10% below aggregator (negative deviation).
    // Aggregator: $1.00, Safe: $0.90.
    t.set_safe_price("USDC", usd_cents(90), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Beyond second tolerance in the negative direction: blocked.
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}
