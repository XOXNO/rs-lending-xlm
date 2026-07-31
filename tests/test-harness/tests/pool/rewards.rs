use controller::constants::RAY;
use test_harness::{
    days, eth_preset, hub_asset, usdc_preset, LendingTest, ALICE, BOB, CAROL, DAVE,
};

fn get_indexes(t: &LendingTest, asset: &str) -> (i128, i128) {
    let asset_addr = t.resolve_asset(asset);
    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset_addr)]);
    let idx = ctrl.get_market_indexes_detailed(&assets).get(0).unwrap();
    (idx.supply_index, idx.borrow_index)
}

#[test]
fn test_add_rewards_index_increase_matches_formula() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let (si_before, _) = get_indexes(&t, "USDC");
    assert_eq!(si_before, RAY, "initial supply index should be 1.0 RAY");

    t.add_rewards("USDC", 1_000.0);

    let (si_after, _) = get_indexes(&t, "USDC");

    let expected_index = RAY + RAY * 1_000 / 100_001;
    let diff = (si_after - expected_index).abs();

    assert!(
        diff <= 1,
        "supply index should be ~1 + 1000/100001 RAY after 1% rewards: expected={}, actual={}, diff={}",
        expected_index,
        si_after,
        diff
    );
}

#[test]
fn test_add_rewards_distributed_proportionally() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 60_000.0);
    t.supply(BOB, "USDC", 40_000.0);

    let alice_before = t.supply_balance(ALICE, "USDC");
    let bob_before = t.supply_balance(BOB, "USDC");

    t.add_rewards("USDC", 10_000.0);

    let alice_after = t.supply_balance(ALICE, "USDC");
    let bob_after = t.supply_balance(BOB, "USDC");

    let alice_reward = alice_after - alice_before;
    let bob_reward = bob_after - bob_before;

    assert!(
        (alice_reward - 6_000.0).abs() < 1.0,
        "Alice (60%) should get ~6,000 of 10,000 rewards, got {:.2}",
        alice_reward
    );

    assert!(
        (bob_reward - 4_000.0).abs() < 1.0,
        "Bob (40%) should get ~4,000 of 10,000 rewards, got {:.2}",
        bob_reward
    );

    let total_distributed = alice_reward + bob_reward;
    assert!(
        (total_distributed - 10_000.0).abs() < 2.0,
        "total distributed should be ~10,000, got {:.2}",
        total_distributed
    );
}

#[test]
fn test_add_rewards_does_not_affect_borrow_index() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.advance_and_sync(days(1));

    let (_, bi_before) = get_indexes(&t, "USDC");

    t.add_rewards("USDC", 50_000.0);

    let (_, bi_after) = get_indexes(&t, "USDC");

    let bi_change_pct = ((bi_after as f64 / bi_before as f64) - 1.0) * 100.0;
    assert!(
        bi_change_pct < 0.01,
        "borrow index should barely change from add_rewards: {:.6}% change",
        bi_change_pct
    );
}

#[test]
fn test_add_rewards_compounds_over_multiple_calls() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let balance_start = t.supply_balance(ALICE, "USDC");

    t.add_rewards("USDC", 1_000.0);
    let after_1 = t.supply_balance(ALICE, "USDC");

    t.add_rewards("USDC", 1_010.0);
    let after_2 = t.supply_balance(ALICE, "USDC");

    t.add_rewards("USDC", 1_020.1);
    let after_3 = t.supply_balance(ALICE, "USDC");

    assert!(
        after_1 > balance_start,
        "1st reward should increase balance"
    );
    assert!(after_2 > after_1, "2nd reward should increase balance");
    assert!(after_3 > after_2, "3rd reward should increase balance");

    let expected = 100_000.0 + 1_000.0 + 1_010.0 + 1_020.1;
    assert!(
        (after_3 - expected).abs() < 5.0,
        "compounded rewards should total ~{:.1}, got {:.2}",
        expected,
        after_3
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #37)")]
fn test_add_rewards_rejects_when_no_supply() {
    let t = LendingTest::new().with_market(usdc_preset()).build();

    t.add_rewards("USDC", 1_000.0);
}

#[test]
fn test_rewards_plus_interest_compound() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.borrow(BOB, "USDC", 50_000.0);

    let balance_before = t.supply_balance(ALICE, "USDC");

    t.advance_and_sync(days(180));
    let balance_after_interest = t.supply_balance(ALICE, "USDC");
    let interest_earned = balance_after_interest - balance_before;

    t.add_rewards("USDC", 5_000.0);
    let balance_after_rewards = t.supply_balance(ALICE, "USDC");
    let reward_earned = balance_after_rewards - balance_after_interest;

    assert!(
        interest_earned > 0.0,
        "should earn interest from borrows: {:.2}",
        interest_earned
    );

    assert!(
        (reward_earned - 5_000.0).abs() < 10.0,
        "rewards should add ~5,000: got {:.2}",
        reward_earned
    );

    let total_gained = balance_after_rewards - balance_before;
    assert!(
        (total_gained - (interest_earned + reward_earned)).abs() < 1.0,
        "total gain should equal interest + rewards"
    );
}

#[test]
fn test_large_rewards_accounting_stable() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 1_000.0);

    t.add_rewards("USDC", 100_000.0);

    let balance = t.supply_balance(ALICE, "USDC");

    assert!(
        (balance - 100_900.0).abs() < 10.0,
        "balance should be ~100,900 after 100x rewards (offset-diluted): got {:.2}",
        balance
    );

    let (si, _) = get_indexes(&t, "USDC");
    let expected_si = RAY * 101_001 / 1_001;
    let diff_pct = ((si as f64 / expected_si as f64) - 1.0).abs() * 100.0;
    assert!(
        diff_pct < 0.1,
        "supply index should be ~100.9 RAY: expected={}, actual={}, diff={:.4}%",
        expected_si,
        si,
        diff_pct
    );
}

#[test]
fn test_four_suppliers_exact_proportional_split() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(BOB, "USDC", 20_000.0);
    t.supply(CAROL, "USDC", 30_000.0);
    t.supply(DAVE, "USDC", 40_000.0);

    let a_before = t.supply_balance(ALICE, "USDC");
    let b_before = t.supply_balance(BOB, "USDC");
    let c_before = t.supply_balance(CAROL, "USDC");
    let d_before = t.supply_balance(DAVE, "USDC");

    t.add_rewards("USDC", 10_000.0);

    let a_reward = t.supply_balance(ALICE, "USDC") - a_before;
    let b_reward = t.supply_balance(BOB, "USDC") - b_before;
    let c_reward = t.supply_balance(CAROL, "USDC") - c_before;
    let d_reward = t.supply_balance(DAVE, "USDC") - d_before;

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

    let total = a_reward + b_reward + c_reward + d_reward;
    assert!(
        (total - 10_000.0).abs() < 5.0,
        "total distributed should be ~10,000: {:.2}",
        total
    );
}

#[test]
fn test_rewards_after_interest_proportional() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 75_000.0);
    t.supply(BOB, "USDC", 25_000.0);

    t.supply(CAROL, "ETH", 100.0);
    t.borrow(CAROL, "USDC", 50_000.0);

    t.advance_and_sync(days(90));

    let a_before_reward = t.supply_balance(ALICE, "USDC");
    let b_before_reward = t.supply_balance(BOB, "USDC");

    t.add_rewards("USDC", 10_000.0);

    let a_reward = t.supply_balance(ALICE, "USDC") - a_before_reward;
    let b_reward = t.supply_balance(BOB, "USDC") - b_before_reward;

    let ratio = a_reward / b_reward;
    assert!(
        (ratio - 3.0).abs() < 0.05,
        "reward split should maintain 3:1 ratio after interest: {:.4}",
        ratio
    );
}
