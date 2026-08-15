use super::{enable_dual_source, setup};
use test_harness::{assert_contract_error, errors, usd, usd_cents, usd_frac, ALICE};

/// One bp past the upper band: the reject side of the boundary the accept
/// test below pins from within.
#[test]
fn test_tolerance_rejects_one_bp_above_the_upper_boundary() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("ETH", usd(2000));
    t.supply(ALICE, "USDC", 100_000.0);

    t.set_safe_price("USDC", usd_frac(10_501, 10_000));
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

/// The lower band edge (reciprocal of 10_500 = 9_524 bps): accepted exactly
/// at the edge, rejected one bp below it.
#[test]
fn test_tolerance_lower_boundary_accepts_at_edge_and_rejects_one_bp_below() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("ETH", usd(2000));
    t.supply(ALICE, "USDC", 100_000.0);

    t.set_safe_price("USDC", usd_frac(9_524, 10_000));
    t.try_borrow(ALICE, "ETH", 5.0)
        .expect("a price exactly at the lower band edge must be accepted");

    t.set_safe_price("USDC", usd_frac(9_523, 10_000));
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

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
