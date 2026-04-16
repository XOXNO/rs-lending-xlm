extern crate std;

use common::constants::RAY;
use test_harness::{days, eth_preset, usdc_preset, LendingTest, ALICE, BOB, CAROL, DAVE};

// ===========================================================================
// Rigorous add_rewards tests: verify the supply index math.
//
// Formula: new_index = old_index * (1 + rewards / total_supplied_value),
// where total_supplied_value = supplied_scaled * old_index / RAY.
//
// Key properties:
// 1. Supply index rises by exactly rewards / total_supplied_value.
// 2. Each supplier's balance rises in proportion to their share.
// 3. Borrow index stays untouched by add_rewards.
// 4. Multiple add_rewards calls compound correctly.
// 5. Rewards with zero supply are a no-op.
// ===========================================================================

fn get_indexes(t: &LendingTest, asset: &str) -> (i128, i128) {
    let asset_addr = t.resolve_asset(asset);
    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::Vec::from_array(&t.env, [asset_addr]);
    let idx = ctrl
        .get_all_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    (idx.supply_index_ray, idx.borrow_index_ray)
}

// ---------------------------------------------------------------------------
// 1. Supply index increases by correct ratio after add_rewards
// ---------------------------------------------------------------------------

#[test]
fn test_add_rewards_index_increase_matches_formula() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Supply 100,000 USDC (7 decimals = 100_000_0000000 units).
    t.supply(ALICE, "USDC", 100_000.0);

    let (si_before, _) = get_indexes(&t, "USDC");
    assert_eq!(si_before, RAY, "initial supply index should be 1.0 RAY");

    // Add 1,000 USDC rewards (1% of 100,000 supply).
    t.add_rewards("USDC", 1_000.0);

    let (si_after, _) = get_indexes(&t, "USDC");

    // Expected: new_index = 1.0 * (1 + 1000/100000) = 1.0 * 1.01 = 1.01 RAY.
    let expected_index = RAY + RAY / 100; // 1.01 * RAY
    let diff = (si_after - expected_index).abs();

    // Allow 1 unit of rounding error (half-up rounding).
    assert!(
        diff <= 1,
        "supply index should be ~1.01 RAY after 1% rewards: expected={}, actual={}, diff={}",
        expected_index,
        si_after,
        diff
    );
}

// ---------------------------------------------------------------------------
// 2. Each supplier gets rewards proportional to their share
// ---------------------------------------------------------------------------

#[test]
fn test_add_rewards_distributed_proportionally() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Alice supplies 60%, Bob supplies 40%.
    t.supply(ALICE, "USDC", 60_000.0);
    t.supply(BOB, "USDC", 40_000.0);

    let alice_before = t.supply_balance(ALICE, "USDC");
    let bob_before = t.supply_balance(BOB, "USDC");

    // Add 10,000 USDC rewards (10% of total supply).
    t.add_rewards("USDC", 10_000.0);

    let alice_after = t.supply_balance(ALICE, "USDC");
    let bob_after = t.supply_balance(BOB, "USDC");

    let alice_reward = alice_after - alice_before;
    let bob_reward = bob_after - bob_before;

    // Alice gets 60% of 10,000 = 6,000.
    assert!(
        (alice_reward - 6_000.0).abs() < 1.0,
        "Alice (60%) should get ~6,000 of 10,000 rewards, got {:.2}",
        alice_reward
    );

    // Bob gets 40% of 10,000 = 4,000.
    assert!(
        (bob_reward - 4_000.0).abs() < 1.0,
        "Bob (40%) should get ~4,000 of 10,000 rewards, got {:.2}",
        bob_reward
    );

    // Total distributed must equal the amount added.
    let total_distributed = alice_reward + bob_reward;
    assert!(
        (total_distributed - 10_000.0).abs() < 2.0,
        "total distributed should be ~10,000, got {:.2}",
        total_distributed
    );
}

// ---------------------------------------------------------------------------
// 3. Borrow index is NOT affected by add_rewards
// ---------------------------------------------------------------------------

#[test]
fn test_add_rewards_does_not_affect_borrow_index() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Sync indexes first so they are clean.
    t.advance_and_sync(days(1));

    let (_, bi_before) = get_indexes(&t, "USDC");

    // Add large rewards to the USDC pool.
    t.add_rewards("USDC", 50_000.0);

    let (_, bi_after) = get_indexes(&t, "USDC");

    // `add_rewards` must not materially change the borrow index. The call
    // runs `global_sync`, but the extra drift is negligible.
    let bi_change_pct = ((bi_after as f64 / bi_before as f64) - 1.0) * 100.0;
    assert!(
        bi_change_pct < 0.01,
        "borrow index should barely change from add_rewards: {:.6}% change",
        bi_change_pct
    );
}

// ---------------------------------------------------------------------------
// 4. Multiple add_rewards calls compound correctly
// ---------------------------------------------------------------------------

#[test]
fn test_add_rewards_compounds_over_multiple_calls() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let balance_start = t.supply_balance(ALICE, "USDC");

    // Add 1% rewards three times.
    t.add_rewards("USDC", 1_000.0); // 1% of 100,000
    let after_1 = t.supply_balance(ALICE, "USDC");

    t.add_rewards("USDC", 1_010.0); // ~1% of 101,000
    let after_2 = t.supply_balance(ALICE, "USDC");

    t.add_rewards("USDC", 1_020.1); // ~1% of 102,010
    let after_3 = t.supply_balance(ALICE, "USDC");

    // Each addition must raise the balance.
    assert!(
        after_1 > balance_start,
        "1st reward should increase balance"
    );
    assert!(after_2 > after_1, "2nd reward should increase balance");
    assert!(after_3 > after_2, "3rd reward should increase balance");

    // Total: roughly 100,000 * 1.01^3 ~= 103,030.1.
    let expected = 100_000.0 + 1_000.0 + 1_010.0 + 1_020.1;
    assert!(
        (after_3 - expected).abs() < 5.0,
        "compounded rewards should total ~{:.1}, got {:.2}",
        expected,
        after_3
    );
}

// ---------------------------------------------------------------------------
// 5. Rewards with zero supply are a no-op (index unchanged)
// ---------------------------------------------------------------------------

#[test]
fn test_add_rewards_noop_when_no_supply() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    // No one has supplied: supply_scaled = 0.
    let (si_before, _) = get_indexes(&t, "USDC");

    // With zero supply, `add_rewards` transfers tokens but must leave the
    // supply index unchanged.
    t.add_rewards("USDC", 1_000.0);

    let (si_after, _) = get_indexes(&t, "USDC");

    // The index must stay unchanged: rewards distributed to zero suppliers
    // are a no-op.
    assert_eq!(
        si_before, si_after,
        "supply index should not change when there are no suppliers"
    );
}

// ---------------------------------------------------------------------------
// 6. Rewards + interest compound together correctly
// ---------------------------------------------------------------------------

#[test]
fn test_rewards_plus_interest_compound() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.borrow(BOB, "USDC", 50_000.0); // 50% utilization

    let balance_before = t.supply_balance(ALICE, "USDC");

    // Accrue interest for 6 months.
    t.advance_and_sync(days(180));
    let balance_after_interest = t.supply_balance(ALICE, "USDC");
    let interest_earned = balance_after_interest - balance_before;

    // Add external rewards on top of interest.
    t.add_rewards("USDC", 5_000.0);
    let balance_after_rewards = t.supply_balance(ALICE, "USDC");
    let reward_earned = balance_after_rewards - balance_after_interest;

    // Interest must be positive (from borrows).
    assert!(
        interest_earned > 0.0,
        "should earn interest from borrows: {:.2}",
        interest_earned
    );

    // Rewards must add ~5,000 to Alice (the sole USDC supplier).
    assert!(
        (reward_earned - 5_000.0).abs() < 10.0,
        "rewards should add ~5,000: got {:.2}",
        reward_earned
    );

    // Total = interest + rewards.
    let total_gained = balance_after_rewards - balance_before;
    assert!(
        (total_gained - (interest_earned + reward_earned)).abs() < 1.0,
        "total gain should equal interest + rewards"
    );
}

// ---------------------------------------------------------------------------
// 7. Large rewards don't break accounting
// ---------------------------------------------------------------------------

#[test]
fn test_large_rewards_accounting_stable() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 1_000.0);

    // Add rewards 100x the supply (extreme case).
    t.add_rewards("USDC", 100_000.0);

    let balance = t.supply_balance(ALICE, "USDC");

    // Balance: ~101,000 (1,000 supply + 100,000 rewards).
    assert!(
        (balance - 101_000.0).abs() < 10.0,
        "balance should be ~101,000 after 100x rewards: got {:.2}",
        balance
    );

    // Supply index: 101x.
    let (si, _) = get_indexes(&t, "USDC");
    let expected_si = RAY * 101; // 101.0 RAY
    let diff_pct = ((si as f64 / expected_si as f64) - 1.0).abs() * 100.0;
    assert!(
        diff_pct < 0.1,
        "supply index should be ~101 RAY: expected={}, actual={}, diff={:.4}%",
        expected_si,
        si,
        diff_pct
    );
}

// ---------------------------------------------------------------------------
// 8. Four suppliers with different shares get exact proportional rewards
// ---------------------------------------------------------------------------

#[test]
fn test_four_suppliers_exact_proportional_split() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // 10% / 20% / 30% / 40% split.
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(BOB, "USDC", 20_000.0);
    t.supply(CAROL, "USDC", 30_000.0);
    t.supply(DAVE, "USDC", 40_000.0);

    let a_before = t.supply_balance(ALICE, "USDC");
    let b_before = t.supply_balance(BOB, "USDC");
    let c_before = t.supply_balance(CAROL, "USDC");
    let d_before = t.supply_balance(DAVE, "USDC");

    // Add exactly 10,000 USDC rewards.
    t.add_rewards("USDC", 10_000.0);

    let a_reward = t.supply_balance(ALICE, "USDC") - a_before;
    let b_reward = t.supply_balance(BOB, "USDC") - b_before;
    let c_reward = t.supply_balance(CAROL, "USDC") - c_before;
    let d_reward = t.supply_balance(DAVE, "USDC") - d_before;

    // Expected: 1000, 2000, 3000, 4000.
    assert!(
        (a_reward - 1_000.0).abs() < 1.0,
        "Alice (10%) should get ~1,000: {:.2}",
        a_reward
    );
    assert!(
        (b_reward - 2_000.0).abs() < 1.0,
        "Bob (20%) should get ~2,000: {:.2}",
        b_reward
    );
    assert!(
        (c_reward - 3_000.0).abs() < 1.0,
        "Carol (30%) should get ~3,000: {:.2}",
        c_reward
    );
    assert!(
        (d_reward - 4_000.0).abs() < 1.0,
        "Dave (40%) should get ~4,000: {:.2}",
        d_reward
    );

    // Conservation: total rewards distributed = total added.
    let total = a_reward + b_reward + c_reward + d_reward;
    assert!(
        (total - 10_000.0).abs() < 5.0,
        "total distributed should be ~10,000: {:.2}",
        total
    );
}

// ---------------------------------------------------------------------------
// 9. Rewards after interest still distribute correctly
// ---------------------------------------------------------------------------

#[test]
fn test_rewards_after_interest_proportional() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice 75%, Bob 25%.
    t.supply(ALICE, "USDC", 75_000.0);
    t.supply(BOB, "USDC", 25_000.0);

    // Create utilization to generate interest.
    t.supply(CAROL, "ETH", 100.0);
    t.borrow(CAROL, "USDC", 50_000.0);

    // Let interest accrue for 90 days.
    t.advance_and_sync(days(90));

    // Balances are no longer exactly 75k/25k due to interest.
    let a_before_reward = t.supply_balance(ALICE, "USDC");
    let b_before_reward = t.supply_balance(BOB, "USDC");

    // Add rewards: must still split proportionally on the current share.
    t.add_rewards("USDC", 10_000.0);

    let a_reward = t.supply_balance(ALICE, "USDC") - a_before_reward;
    let b_reward = t.supply_balance(BOB, "USDC") - b_before_reward;

    // Ratio remains 3:1 (proportional to scaled amounts, which are unchanged).
    let ratio = a_reward / b_reward;
    assert!(
        (ratio - 3.0).abs() < 0.05,
        "reward split should maintain 3:1 ratio after interest: {:.4}",
        ratio
    );
}
