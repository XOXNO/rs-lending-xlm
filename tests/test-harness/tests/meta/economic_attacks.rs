use test_harness::{helpers, usd_cents, LendingTest, ALICE, BOB, LIQUIDATOR};

#[test]
fn test_donation_attack_does_not_inflate_share_price() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 11.0);
    let alice_supply_before = t.supply_balance(ALICE, "USDC");

    let pool_addr = t.resolve_market("USDC").pool.clone();
    let market = t.resolve_market("USDC");
    let amount_raw = helpers::f64_to_i128(1_000_000.0, market.decimals);
    market.token_admin.mint(&pool_addr, &amount_raw);

    t.supply(BOB, "USDC", 1_000.0);
    let bob_supply_after = t.supply_balance(BOB, "USDC");
    assert!(
        bob_supply_after > 999.0 && bob_supply_after < 1_001.0,
        "victim deposit must be credited at face value despite donation; got {:.4}",
        bob_supply_after
    );

    let alice_supply_after = t.supply_balance(ALICE, "USDC");
    assert!(
        (alice_supply_after - alice_supply_before).abs() < 0.01,
        "attacker's supply should not grow from donation; before={:.4} after={:.4}",
        alice_supply_before,
        alice_supply_after
    );
}

#[test]
fn test_first_supplier_cannot_dilute_followers() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 11.0);
    let alice_after = t.supply_balance(ALICE, "USDC");

    t.supply(BOB, "USDC", 50_000.0);
    let bob_after = t.supply_balance(BOB, "USDC");

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

#[test]
fn test_partial_liquidation_chain_converges() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    t.set_price("USDC", usd_cents(74));
    t.assert_liquidatable(ALICE);

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
        iters < max_iters,
        "partial-liquidation chain must exit by convergence (healthy or dust), \
         not by exhausting the {max_iters}-iteration budget",
    );

    let final_hf_safe = !t.can_be_liquidated(ALICE);
    let position_closed = t.borrow_balance(ALICE, "ETH") < 0.001;
    assert!(
        final_hf_safe || position_closed,
        "convergence requires either HF safe or position closed; final iters={}",
        iters
    );
}
