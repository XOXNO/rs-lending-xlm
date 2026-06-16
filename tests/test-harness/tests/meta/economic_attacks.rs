use test_harness::{
    eth_preset, helpers, usd_cents, usdc_preset, LendingTest, ALICE, BOB, LIQUIDATOR,
};
// 1. Donation attack defense

// Classic share-inflation attack: an attacker supplies a single dust
// share, then donates a huge amount of underlying directly to the pool
// to inflate the share price. The next depositor's deposit gets
// rounded down to ~0 shares.
//
// In this protocol the supply ledger is index-driven (`supplied_ray`
// stored as a rate-tracked total, not a token-balance lookup) — a raw
// token mint to the pool address does not affect the supply tracker,
// so the share price stays constant and victim's supply is preserved.
#[test]
fn test_donation_attack_does_not_inflate_share_price() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Attacker supplies the minimum allowed amount (just above the
    // $10 dust floor) — as small a seed as the protocol permits.
    t.supply(ALICE, "USDC", 11.0);
    let alice_supply_before = t.supply_balance(ALICE, "USDC");

    // Direct donation to the pool: mint tokens to the pool address
    // bypassing the supply entrypoint. Use a large amount relative to
    // attacker's supply to maximize the inflation effect a vulnerable
    // protocol would suffer.
    let pool_addr = t.resolve_market("USDC").pool.clone();
    let market = t.resolve_market("USDC");
    let amount_raw = helpers::f64_to_i128(1_000_000.0, market.decimals);
    market.token_admin.mint(&pool_addr, &amount_raw);

    // Victim deposits — must receive credit roughly proportional to
    // their deposit, not be rounded to zero.
    t.supply(BOB, "USDC", 1_000.0);
    let bob_supply_after = t.supply_balance(BOB, "USDC");
    assert!(
        bob_supply_after > 999.0 && bob_supply_after < 1_001.0,
        "victim deposit must be credited at face value despite donation; got {:.4}",
        bob_supply_after
    );

    // Attacker's prior position is unchanged in supply units (the
    // donation became pool dust that the protocol treats as
    // operator revenue, not supply).
    let alice_supply_after = t.supply_balance(ALICE, "USDC");
    assert!(
        (alice_supply_after - alice_supply_before).abs() < 0.01,
        "attacker's supply should not grow from donation; before={:.4} after={:.4}",
        alice_supply_before,
        alice_supply_after
    );
}
// 2. First-supplier cannot dilute later suppliers via tiny initial seed

// Pins that two supplies in sequence preserve proportional credit.
// Without correct share math, a $1 first supply followed by a $10000
// second supply could leave the second supplier with 0 shares.
#[test]
fn test_first_supplier_cannot_dilute_followers() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // First supplier seeds with the protocol minimum ($11 ≈ dust
    // floor + 1) — the smallest seed the protocol will accept.
    t.supply(ALICE, "USDC", 11.0);
    let alice_after = t.supply_balance(ALICE, "USDC");

    // Second supplier supplies a much larger amount.
    t.supply(BOB, "USDC", 50_000.0);
    let bob_after = t.supply_balance(BOB, "USDC");

    // Each must hold approximately the amount they supplied — no
    // disproportionate dilution.
    assert!(
        (alice_after - 11.0).abs() < 0.01,
        "first supplier holds ~$11: got {:.4}",
        alice_after
    );
    assert!(
        (bob_after - 50_000.0).abs() < 1.0,
        "second supplier holds ~$50000: got {:.4}",
        bob_after
    );
}
// 3. Partial-liquidation chain converges (no infinite-loop griefing)

// A series of partial liquidations on a single underwater account must
// converge: each partial payment increases HF (or cleans up dust), and
// after a bounded number of partials the position is either healthy
// or has been fully closed. Without this convergence, a malicious
// liquidator could perpetually leave the position 1 ulp underwater.
#[test]
fn test_partial_liquidation_chain_converges() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0); // $6000 debt
                                 // Shallow crash → HF ≈ 0.97. Within a couple of partials the
                                 // position should be lifted back to safe.
    t.set_price("USDC", usd_cents(74));
    t.assert_liquidatable(ALICE);

    // Up to 12 partial liquidations of 0.5 ETH each. Each must succeed
    // until HF goes back above 1.0 or the position is closed.
    let mut iters = 0u32;
    let max_iters = 12u32;
    while t.can_be_liquidated(ALICE) && iters < max_iters {
        if t.borrow_balance(ALICE, "ETH") < 0.001 {
            break;
        }
        t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
        iters += 1;
    }

    assert!(
        iters <= max_iters,
        "partial-liquidation chain must converge within {} iters; took {}",
        max_iters,
        iters
    );
    // Final state: either healthy or bad-debt-cleared.
    let final_hf_safe = !t.can_be_liquidated(ALICE);
    let position_closed = t.borrow_balance(ALICE, "ETH") < 0.001;
    assert!(
        final_hf_safe || position_closed,
        "convergence requires either HF safe or position closed; final iters={}",
        iters
    );
}
