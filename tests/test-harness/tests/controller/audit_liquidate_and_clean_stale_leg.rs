use test_harness::{
    errors, eth_preset, usd, usd_cents, usdc_preset, wbtc_preset, LendingTest, LIQUIDATOR,
};

#[test]
fn audit_liquidate_and_clean_bricked_by_unpriceable_dust_leg() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.set_oracle_single_spot("WBTC");

    t.supply(LIQUIDATOR, "USDC", 50_000.0);
    let borrower = test_harness::ALICE;
    t.supply(borrower, "USDC", 10_000.0);
    t.borrow(borrower, "ETH", 3.0);

    let fresh = t.try_liquidate(LIQUIDATOR, borrower, "ETH", 1.0);
    test_harness::assert_contract_error(fresh, errors::HEALTH_FACTOR_TOO_HIGH);

    t.advance_time(5_000);
    let now = t.env.ledger().timestamp();
    let wbtc = t.resolve_asset("WBTC");
    t.mock_reflector_client()
        .set_price_at(&wbtc, &usd(60_000), &(now - 3_600));

    let plant = t.try_supply(borrower, "WBTC", 0.001);
    assert!(
        plant.is_ok(),
        "supply must accept the fragile leg with a stale feed: {plant:?}"
    );

    t.set_price("USDC", usd_cents(50));

    let borrower_id = t.resolve_account_id(borrower);

    let liq = t.try_liquidate(LIQUIDATOR, borrower, "ETH", 1.0);
    test_harness::assert_contract_error(liq, errors::PRICE_FEED_STALE);

    let clean = t.try_clean_bad_debt_by_id(borrower_id);
    test_harness::assert_contract_error(clean, errors::PRICE_FEED_STALE);

    t.set_price("WBTC", usd(60_000));
    let recovered = t.try_liquidate(LIQUIDATOR, borrower, "ETH", 1.0);
    assert!(
        recovered.is_ok(),
        "once WBTC is fresh, the identical liquidation must succeed: {recovered:?}"
    );
    assert!(
        t.borrow_balance(borrower, "ETH") < 3.0,
        "post-recovery liquidation must reduce the borrower's debt"
    );
}
