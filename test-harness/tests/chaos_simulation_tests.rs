extern crate std;

use common::constants::RAY;
use test_harness::{
    days, eth_preset, usd, usdc_preset, wbtc_preset, LendingTest, ALICE, BOB, CAROL, DAVE, EVE,
    LIQUIDATOR,
};

// ---------------------------------------------------------------------------
// Helpers: deterministic pseudo-random (no std::rand in soroban)
// ---------------------------------------------------------------------------

/// Simple LCG for deterministic "randomness" in tests.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    fn range(&mut self, min: u64, max: u64) -> u64 {
        min + (self.next() % (max - min + 1))
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        let idx = self.next() as usize % items.len();
        &items[idx]
    }
}

// ---------------------------------------------------------------------------
// 1. Chaos: 15 users, random valid operations over 8 weeks, invariant check
// ---------------------------------------------------------------------------

#[test]
fn test_chaos_multi_user_random_operations() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    let users = [
        "u01", "u02", "u03", "u04", "u05", "u06", "u07", "u08", "u09", "u10", "u11", "u12", "u13",
        "u14", "u15",
    ];
    let supply_assets = ["USDC", "ETH", "WBTC"];
    let borrow_assets = ["USDC", "ETH", "WBTC"];

    let mut rng = Rng::new(42);

    // Phase 1: All users supply random assets with random amounts
    for user in &users {
        let asset = *rng.pick(&supply_assets);
        let amount = match asset {
            "USDC" => rng.range(5_000, 100_000) as f64,
            "ETH" => rng.range(1, 20) as f64,
            "WBTC" => rng.range(1, 5) as f64 * 0.1,
            _ => unreachable!(),
        };
        t.supply(user, asset, amount);
    }

    // Phase 2: Half the users borrow (conservative amounts)
    let mut borrow_successes = 0u32;
    let mut borrow_failures = 0u32;
    for user in &users[0..8] {
        let asset = *rng.pick(&borrow_assets);
        // Borrow ~20% of collateral value (very safe, well within 75% LTV)
        let amount = match asset {
            "USDC" => rng.range(500, 5_000) as f64,
            "ETH" => rng.range(1, 3) as f64 * 0.1,
            "WBTC" => rng.range(1, 5) as f64 * 0.001,
            _ => unreachable!(),
        };
        // Track successes vs failures from insufficient collateral
        match t.try_borrow(user, asset, amount) {
            Ok(_) => borrow_successes += 1,
            Err(_) => borrow_failures += 1,
        }
    }

    // Advance 1 week + sync
    t.advance_and_sync(days(7));

    // Phase 3: Some partial repays and additional borrows
    for user in users.iter().take(5) {
        let user = *user;
        let asset = *rng.pick(&borrow_assets);
        // Small repays may fail if the sampled asset is not borrowed by that
        // user, so the result is ignored here.
        let _ = t.try_repay(user, asset, 100.0);
    }

    // Advance another week
    t.advance_and_sync(days(7));

    // Phase 4: Price movement — ETH drops 10%
    t.set_price("ETH", usd(1800));
    t.advance_and_sync(days(7));

    // Phase 5: More activity
    for user in &users[8..12] {
        let user = *user;
        let asset = *rng.pick(&borrow_assets);
        let amount = match asset {
            "USDC" => rng.range(100, 2_000) as f64,
            "ETH" => rng.range(1, 2) as f64 * 0.05,
            "WBTC" => rng.range(1, 3) as f64 * 0.001,
            _ => unreachable!(),
        };
        match t.try_borrow(user, asset, amount) {
            Ok(_) => borrow_successes += 1,
            Err(_) => borrow_failures += 1,
        }
    }

    // Advance final weeks
    t.advance_and_sync(days(7));
    t.advance_and_sync(days(7));

    // Restore price
    t.set_price("ETH", usd(2000));

    // -----------------------------------------------------------------------
    // OPERATION SUCCESS TRACKING
    // -----------------------------------------------------------------------

    // All 15 supplies should succeed (Phase 1 uses safe amounts)
    // At least some borrows and repays should succeed
    assert!(
        borrow_successes >= 3,
        "at least 3 of 12 borrows should succeed, got {} successes / {} failures",
        borrow_successes,
        borrow_failures
    );

    // -----------------------------------------------------------------------
    // INVARIANT CHECKS
    // -----------------------------------------------------------------------

    // 1. All accounts with borrows must have HF >= 1.0 (or be cleaned up)
    for user in &users {
        if let Some(user_state) = t.users.get(*user) {
            if user_state.default_account_id.is_some() {
                let hf = t.health_factor(user);
                // HF >= 1.0 or user has no borrows (HF = max)
                assert!(
                    hf >= 1.0 || hf == f64::MAX || hf > 1e18,
                    "user {} HF should be >= 1.0, got {}",
                    user,
                    hf
                );
            }
        }
    }

    // 2. Supply and borrow indexes must have increased from 1.0 RAY
    for asset in &["USDC", "ETH", "WBTC"] {
        let asset_addr = t.resolve_asset(asset);
        let ctrl = t.ctrl_client();
        let assets = soroban_sdk::Vec::from_array(&t.env, [asset_addr]);
        let index = ctrl
            .get_all_market_indexes_detailed(&assets)
            .get(0)
            .unwrap();
        assert!(
            index.supply_index_ray >= RAY,
            "{} supply index should be >= 1.0 RAY",
            asset
        );
        assert!(
            index.borrow_index_ray >= RAY,
            "{} borrow index should be >= 1.0 RAY",
            asset
        );
    }

    // 3. Protocol revenue should be >= 0 for all markets
    for asset in &["USDC", "ETH", "WBTC"] {
        let rev = t.snapshot_revenue(asset);
        assert!(rev >= 0, "{} revenue should be >= 0, got {}", asset, rev);
    }
}

// ---------------------------------------------------------------------------
// 2. Full bank-run exit: everyone repays and withdraws, protocol solvent
// ---------------------------------------------------------------------------

#[test]
fn test_chaos_bank_run_full_exit() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let suppliers = [ALICE, BOB, CAROL, DAVE, EVE];
    let _borrowers = [ALICE, BOB, CAROL];

    // Setup: everyone supplies
    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(BOB, "USDC", 30_000.0);
    t.supply(CAROL, "ETH", 10.0);
    t.supply(DAVE, "ETH", 5.0);
    t.supply(EVE, "USDC", 20_000.0);

    // Some borrow
    t.borrow(ALICE, "ETH", 5.0); // ~$10k vs $50k collateral
    t.borrow(BOB, "ETH", 3.0); // ~$6k vs $30k collateral
    t.borrow(CAROL, "USDC", 5_000.0); // $5k vs $20k collateral

    // Accrue 90 days of interest
    t.advance_and_sync(days(30));
    t.advance_and_sync(days(30));
    t.advance_and_sync(days(30));

    // Snapshot revenue before exit
    let usdc_rev_before = t.snapshot_revenue("USDC");
    let eth_rev_before = t.snapshot_revenue("ETH");

    // BANK RUN: all borrowers repay with massive overpayment (pool refunds excess)
    t.repay(ALICE, "ETH", 100.0); // way more than owed
    t.repay(BOB, "ETH", 100.0);
    t.repay(CAROL, "USDC", 100_000.0);

    // All borrowers should have ~0 debt now
    assert!(
        t.borrow_balance(ALICE, "ETH") < 0.001,
        "Alice debt should be ~0 after full repay"
    );
    assert!(
        t.borrow_balance(BOB, "ETH") < 0.001,
        "Bob debt should be ~0 after full repay"
    );
    assert!(
        t.borrow_balance(CAROL, "USDC") < 0.01,
        "Carol debt should be ~0 after full repay"
    );

    // All suppliers withdraw everything.
    // Track successes: each user should succeed for the asset they supplied.
    let mut withdraw_successes = 0u32;
    for user in &suppliers {
        // Try withdrawing each asset (some won't have positions, that's ok)
        if t.try_withdraw(user, "USDC", 999_999.0).is_ok() {
            withdraw_successes += 1;
        }
        if t.try_withdraw(user, "ETH", 999_999.0).is_ok() {
            withdraw_successes += 1;
        }
    }
    // All 5 suppliers should successfully withdraw from their supplied asset
    assert!(
        withdraw_successes >= 5,
        "all suppliers should successfully withdraw: got {} successes out of 5 suppliers",
        withdraw_successes
    );

    // SOLVENCY CHECK: pool reserves must be >= 0
    let usdc_reserves = t.pool_reserves("USDC");
    let eth_reserves = t.pool_reserves("ETH");
    assert!(
        usdc_reserves >= 0.0,
        "USDC pool should be solvent, reserves={}",
        usdc_reserves
    );
    assert!(
        eth_reserves >= 0.0,
        "ETH pool should be solvent, reserves={}",
        eth_reserves
    );

    // REVENUE CHECK: protocol collected fees
    let usdc_rev_after = t.snapshot_revenue("USDC");
    let eth_rev_after = t.snapshot_revenue("ETH");
    assert!(
        usdc_rev_after >= usdc_rev_before,
        "USDC revenue should not decrease"
    );
    assert!(
        eth_rev_after >= eth_rev_before,
        "ETH revenue should not decrease"
    );
}

// ---------------------------------------------------------------------------
// 3. Sustained high utilization: verify rates stay sane over 1 year
// ---------------------------------------------------------------------------

#[test]
fn test_chaos_sustained_high_utilization() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply + borrow to ~85% utilization (above optimal 80%)
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0); // $200k collateral

    // Borrow ~85% of USDC supply
    t.borrow(BOB, "USDC", 85_000.0);

    let mut prev_debt = t.borrow_balance(BOB, "USDC");
    let mut prev_supply = t.supply_balance(ALICE, "USDC");

    // Simulate 12 months with monthly syncs
    for month in 1..=12 {
        t.advance_and_sync(days(30));

        let new_debt = t.borrow_balance(BOB, "USDC");
        let new_supply = t.supply_balance(ALICE, "USDC");

        // Debt must strictly increase (interest accruing)
        assert!(
            new_debt > prev_debt,
            "month {}: debt should increase: {} -> {}",
            month,
            prev_debt,
            new_debt
        );

        // Supply balance should increase (depositors earn interest)
        assert!(
            new_supply > prev_supply,
            "month {}: supply should increase: {} -> {}",
            month,
            prev_supply,
            new_supply
        );

        prev_debt = new_debt;
        prev_supply = new_supply;
    }

    // After 1 year at 85% utilization, debt should have grown significantly
    // (slope3 kicks in above optimal, ~300% slope)
    let final_debt = t.borrow_balance(BOB, "USDC");
    let growth = final_debt / 85_000.0;
    assert!(
        growth > 1.05,
        "1 year at high utilization should grow debt >5%, actual growth: {:.2}x",
        growth
    );

    // Note: HF may have dropped below 1.0 due to extreme interest accrual.
    // This is correct protocol behavior — the account becomes liquidatable
    // when debt grows past collateral value. A keeper/liquidator would handle this.
    let final_hf = t.health_factor(BOB);
    if final_hf < 1.0 {
        // Account is liquidatable — this is expected at extreme utilization over time
        assert!(
            t.can_be_liquidated(BOB),
            "low HF account should be liquidatable"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Rapid price oscillation: verify no wrongful liquidations
// ---------------------------------------------------------------------------

#[test]
fn test_chaos_price_oscillation_no_wrongful_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply 100k USDC, borrow 10 ETH ($20k) — HF = (100k*0.8)/20k = 4.0
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // Oscillate ETH price: $2000 -> $1500 -> $2500 -> $1800 -> $2200
    let prices = [1500, 2500, 1800, 2200, 2000];
    for price in &prices {
        t.set_price("ETH", usd(*price));
        t.advance_and_sync(days(1));

        // Alice should NEVER be liquidatable with 4x over-collateralization
        // Even at $2500 ETH, debt = $25k, HF = (100k*0.8)/25k = 3.2
        assert!(
            !t.can_be_liquidated(ALICE),
            "well-collateralized account should never be liquidatable at ETH=${}",
            price
        );

        // Try liquidation — should always fail
        let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
        assert!(
            result.is_err(),
            "liquidation should fail on healthy account at ETH=${}",
            price
        );
    }
}

// ---------------------------------------------------------------------------
// 5. Multi-market borrow/repay cycle: accounting consistency
// ---------------------------------------------------------------------------

#[test]
fn test_chaos_multi_market_accounting() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    // Alice supplies all 3 markets
    t.supply(ALICE, "USDC", 200_000.0);
    t.supply(ALICE, "ETH", 10.0);
    t.supply(ALICE, "WBTC", 0.5);

    // Borrow from all 3 markets
    t.borrow(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "WBTC", 0.01);

    let total_collateral_before = t.total_collateral(ALICE);
    let total_debt_before = t.total_debt(ALICE);
    let hf_before = t.health_factor(ALICE);

    // Advance 6 months
    t.advance_and_sync(days(180));

    let total_collateral_after = t.total_collateral(ALICE);
    let total_debt_after = t.total_debt(ALICE);
    let hf_after = t.health_factor(ALICE);

    // Collateral should increase (supply interest)
    assert!(
        total_collateral_after >= total_collateral_before,
        "collateral should not decrease: {} -> {}",
        total_collateral_before,
        total_collateral_after
    );

    // Debt should increase (borrow interest)
    assert!(
        total_debt_after > total_debt_before,
        "debt should grow with interest: {} -> {}",
        total_debt_before,
        total_debt_after
    );

    // HF should decrease (debt grows faster than collateral)
    assert!(
        hf_after < hf_before,
        "HF should decrease as debt grows: {} -> {}",
        hf_before,
        hf_after
    );

    // But should still be healthy (started very over-collateralized)
    t.assert_healthy(ALICE);

    // Full repay cycle
    t.repay(ALICE, "USDC", 999_999.0);
    t.repay(ALICE, "ETH", 999.0);
    t.repay(ALICE, "WBTC", 999.0);

    // After full repay, debt should be ~0
    let final_debt = t.total_debt(ALICE);
    assert!(
        final_debt < 1.0,
        "debt should be ~0 after full repay, got {}",
        final_debt
    );
}

// ---------------------------------------------------------------------------
// 6. Full keeper + revenue lifecycle in simulation
// ---------------------------------------------------------------------------

#[test]
fn test_chaos_keeper_revenue_lifecycle() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Phase 1: Users supply and borrow
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 50.0);
    t.borrow(ALICE, "ETH", 10.0);
    t.borrow(BOB, "USDC", 30_000.0);

    // Phase 2: Keeper updates indexes manually (not via advance_and_sync)
    t.advance_time(days(7));
    t.update_indexes_for(&["USDC", "ETH"]);

    // Indexes should have increased
    let usdc_addr = t.resolve_asset("USDC");
    let eth_addr = t.resolve_asset("ETH");
    let ctrl = t.ctrl_client();
    let usdc_assets = soroban_sdk::Vec::from_array(&t.env, [usdc_addr]);
    let eth_assets = soroban_sdk::Vec::from_array(&t.env, [eth_addr]);
    let usdc_index = ctrl
        .get_all_market_indexes_detailed(&usdc_assets)
        .get(0)
        .unwrap();
    let eth_index = ctrl
        .get_all_market_indexes_detailed(&eth_assets)
        .get(0)
        .unwrap();
    assert!(
        usdc_index.borrow_index_ray > RAY,
        "USDC borrow index should increase"
    );
    assert!(
        eth_index.borrow_index_ray > RAY,
        "ETH borrow index should increase"
    );

    // Phase 3: More time passes, keeper syncs again
    t.advance_time(days(30));
    t.update_indexes_for(&["USDC", "ETH"]);

    // Phase 4: Verify revenue accumulated from interest
    let usdc_rev = t.snapshot_revenue("USDC");
    let eth_rev = t.snapshot_revenue("ETH");
    assert!(
        usdc_rev > 0,
        "USDC should have protocol revenue after 37 days"
    );
    assert!(
        eth_rev > 0,
        "ETH should have protocol revenue after 37 days"
    );

    // Set accumulator address before claiming (controller requires it)
    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.ctrl_client().set_accumulator(&accumulator);

    // Actually claim it
    let claimed_usdc = t.claim_revenue("USDC");
    assert!(
        claimed_usdc > 0,
        "should claim positive USDC revenue: {}",
        claimed_usdc
    );

    let claimed_eth = t.claim_revenue("ETH");
    assert!(
        claimed_eth > 0,
        "should claim positive ETH revenue: {}",
        claimed_eth
    );

    // Phase 5: Add external rewards
    t.add_rewards("USDC", 1_000.0);

    // Alice's USDC supply balance should have increased from rewards
    let alice_supply = t.supply_balance(ALICE, "USDC");
    assert!(
        alice_supply > 100_000.0,
        "Alice supply should exceed initial after rewards: {}",
        alice_supply
    );

    // Phase 6: Continue for 60 more days, then full exit
    t.advance_and_sync(days(60));

    // Full repay
    t.repay(ALICE, "ETH", 999.0);
    t.repay(BOB, "USDC", 999_999.0);

    // Full withdraw -- verify these succeed since users have positions
    let alice_withdraw = t.try_withdraw(ALICE, "USDC", 999_999.0);
    assert!(
        alice_withdraw.is_ok(),
        "Alice should successfully withdraw USDC after full repay"
    );
    let bob_withdraw = t.try_withdraw(BOB, "ETH", 999.0);
    assert!(
        bob_withdraw.is_ok(),
        "Bob should successfully withdraw ETH after full repay"
    );

    // Solvency invariant
    assert!(t.pool_reserves("USDC") >= 0.0, "USDC pool solvent");
    assert!(t.pool_reserves("ETH") >= 0.0, "ETH pool solvent");
}
