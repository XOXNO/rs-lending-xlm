use super::{enable_dual_source, setup};
use test_harness::{assert_contract_error, errors, usd, usd_cents, ALICE};

#[test]
fn test_tolerance_at_exact_upper_boundary() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd_cents(105));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);

    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(
        result.is_ok(),
        "borrow should work at the tolerance boundary"
    );
}

#[test]
fn test_tolerance_inside_single_band() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd_cents(104));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);

    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(
        result.is_ok(),
        "borrow should work inside the tolerance band"
    );
}

#[test]
fn test_safe_price_below_aggregator_blocks_borrow() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd_cents(90));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);

    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}
