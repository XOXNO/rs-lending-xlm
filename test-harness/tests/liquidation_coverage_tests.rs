extern crate std;

use test_harness::{
    assert_contract_error, eth_preset, usd_cents, usdc_preset, LendingTest, LIQUIDATOR,
};

#[test]
fn test_liquidation_skips_excess_debt_payments() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.set_exchange_source("USDC", common::types::ExchangeSource::SpotOnly);
    t.set_exchange_source("ETH", common::types::ExchangeSource::SpotOnly);

    let alice = "alice_excess";

    // Alice supplies 100k USDC and borrows 2.5 ETH (~$5000).
    t.supply(alice, "USDC", 100_000.0);
    t.borrow(alice, "ETH", 2.5);

    // Drop USDC to $0.06 => Collateral $6000. Debt $5000.
    // Weighted = 4800. HF = 0.96.
    t.set_price("USDC", usd_cents(6));

    t.assert_liquidatable(alice);

    t.liquidate_multi(LIQUIDATOR, alice, &[("ETH", 2.0), ("ETH", 0.1)]);

    let debt = t.borrow_balance(alice, "ETH");
    assert!(
        debt > 0.0,
        "Alice should have significant debt left, got {}",
        debt
    );
}

// Post-audit (T1-3, M-02): `token_price` now rejects zero oracle prices
// globally. The original scenario (USDC price = 0 -> degenerate HF math ->
// liquidation fails with INVALID_PAYMENTS) is now unreachable. This test
// asserts the safer behavior: reading any price for an asset whose oracle
// returned 0 panics immediately with `OracleError::InvalidPrice` (#217).
#[test]
#[should_panic(expected = "Error(Contract, #217)")]
fn test_liquidation_zero_collateral_proportion() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let alice = "alice_zero";

    t.supply(alice, "USDC", 100.0);
    t.borrow(alice, "ETH", 0.02); // ~$40

    t.set_exchange_source("USDC", common::types::ExchangeSource::SpotOnly);
    t.set_price("USDC", 0);

    // Any price fetch for USDC now panics with InvalidPrice.
    t.assert_liquidatable(alice);
}

#[test]
fn test_liquidation_seize_proportional_dust_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.set_exchange_source("USDC", common::types::ExchangeSource::SpotOnly);
    t.set_exchange_source("ETH", common::types::ExchangeSource::SpotOnly);

    let alice = "alice_dust";

    // Supply 20k USDC for enough collateral to back a 6 ETH borrow.
    t.supply(alice, "USDC", 20_000.0);
    t.supply(alice, "ETH", 0.0000001); // 1 unit
    t.borrow(alice, "ETH", 6.0); // $12,000. Initial weighted = 16,000.

    // Drop USDC to $0.50 => collateral value $10,000, weighted $8,000.
    // Debt = $12,000. HF = 8000/12000 = 0.66.
    t.set_price("USDC", usd_cents(50));

    t.assert_liquidatable(alice);

    t.liquidate(LIQUIDATOR, alice, "ETH", 0.01);

    let eth_bal = t.supply_balance(alice, "ETH");
    assert!(eth_bal > 0.0);
}

#[test]
fn test_liquidation_rejects_if_no_debt_repaid() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.set_exchange_source("USDC", common::types::ExchangeSource::SpotOnly);
    t.set_exchange_source("ETH", common::types::ExchangeSource::SpotOnly);
    t.supply("alice_rej", "USDC", 100.0);
    t.borrow("alice_rej", "ETH", 0.03);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable("alice_rej");

    // Pay with a tiny amount that rescale might turn into 0.
    let result = t.try_liquidate(LIQUIDATOR, "alice_rej", "ETH", 0.000000001);
    assert_contract_error(result, 14); // AmountMustBePositive.
}

#[test]
fn test_liquidation_multi_debt_capped() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.set_exchange_source("USDC", common::types::ExchangeSource::SpotOnly);
    t.set_exchange_source("ETH", common::types::ExchangeSource::SpotOnly);

    let alice = "alice_multi";

    // Alice supplies $1000 USDC, borrows 0.15 ETH (~$300) and $100 USDC.
    // Total debt ~$400.
    t.supply(alice, "USDC", 1000.0);
    t.borrow(alice, "ETH", 0.15);
    t.borrow(alice, "USDC", 100.0);

    // Drop USDC to $0.50 => collateral $500, weighted $400, debt $400.
    // Near boundary; drop just a bit more.
    t.set_price("USDC", usd_cents(40)); // Collateral $400, weighted $320, debt $400, HF = 0.8.

    t.assert_liquidatable(alice);

    // The liquidator pays with two different tokens.
    // Ideal repayment for $400 debt with $320 weighted is:
    // (400 - 320) / (1 - 0.1) = 80 / 0.9 = ~$88.

    // 1. Pay with ETH (0.1 ETH = $200). This fulfills the ideal (~$148).
    // 2. Pay with USDC ($50). The contract must skip this (continue) because remaining_ideal <= 0.

    t.liquidate_multi(LIQUIDATOR, alice, &[("ETH", 0.1), ("USDC", 50.0)]);

    // The ETH debt should have dropped by the capped amount (~$148 worth of ETH).
    let debt_eth = t.borrow_balance(alice, "ETH");
    assert!(debt_eth < 0.15);

    // The USDC debt must remain UNCHANGED because the contract skipped it.
    let debt_usdc = t.borrow_balance(alice, "USDC");
    assert_eq!(debt_usdc, 100.0);
}
