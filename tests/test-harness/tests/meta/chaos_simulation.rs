use controller::constants::RAY;
use test_harness::{
    days, eth_preset, hub_asset, usd, usdc_preset, LendingTest, ALICE, BOB, CAROL, DAVE, EVE,
    LIQUIDATOR,
};

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

#[test]
fn test_chaos_multi_user_seeded_operation_sequence() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    let users = [
        "u01", "u02", "u03", "u04", "u05", "u06", "u07", "u08", "u09", "u10", "u11", "u12", "u13",
        "u14", "u15",
    ];
    let supply_assets = ["USDC", "ETH", "WBTC"];
    let borrow_assets = ["USDC", "ETH", "WBTC"];

    let mut rng = Rng::new(42);

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

    let mut borrow_successes = 0u32;
    let mut borrow_failures = 0u32;
    for user in &users[0..8] {
        let asset = *rng.pick(&borrow_assets);

        let amount = match asset {
            "USDC" => rng.range(500, 5_000) as f64,
            "ETH" => rng.range(1, 3) as f64 * 0.1,
            "WBTC" => rng.range(1, 5) as f64 * 0.001,
            _ => unreachable!(),
        };

        match t.try_borrow(user, asset, amount) {
            Ok(_) => borrow_successes += 1,
            Err(_) => borrow_failures += 1,
        }
    }

    t.advance_and_sync(days(7));

    for user in users.iter().take(5) {
        let user = *user;
        let asset = *rng.pick(&borrow_assets);

        let _ = t.try_repay(user, asset, 100.0);
    }

    t.advance_and_sync(days(7));

    t.set_price("ETH", usd(1800));
    t.advance_and_sync(days(7));

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

    t.advance_and_sync(days(7));
    t.advance_and_sync(days(7));

    t.set_price("ETH", usd(2000));

    assert!(
        borrow_successes >= 3,
        "at least 3 of 12 borrows should succeed, got {} successes / {} failures",
        borrow_successes,
        borrow_failures
    );

    for user in &users {
        if let Some(user_state) = t.users.get(*user) {
            if user_state.default_account_id.is_some() {
                let hf = t.health_factor(user);

                let healthy = hf >= 1.0;
                assert!(healthy, "user {} HF should be >= 1.0, got {}", user, hf);
            }
        }
    }

    for asset in &["USDC", "ETH", "WBTC"] {
        let asset_addr = t.resolve_asset(asset);
        let ctrl = t.ctrl_client();
        let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset_addr)]);
        let index = ctrl.get_market_indexes_detailed(&assets).get(0).unwrap();
        assert!(
            index.supply_index >= RAY,
            "{} supply index should be >= 1.0 RAY",
            asset
        );
        assert!(
            index.borrow_index >= RAY,
            "{} borrow index should be >= 1.0 RAY",
            asset
        );
    }

    for asset in &["USDC", "ETH", "WBTC"] {
        let rev = t.snapshot_revenue(asset);
        assert!(rev >= 0, "{} revenue should be >= 0, got {}", asset, rev);
    }
}

#[test]
fn test_chaos_bank_run_full_exit() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let suppliers = [ALICE, BOB, CAROL, DAVE, EVE];
    let _borrowers = [ALICE, BOB, CAROL];

    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(BOB, "USDC", 30_000.0);
    t.supply(CAROL, "ETH", 10.0);
    t.supply(DAVE, "ETH", 5.0);
    t.supply(EVE, "USDC", 20_000.0);

    t.borrow(ALICE, "ETH", 5.0);
    t.borrow(BOB, "ETH", 3.0);
    t.borrow(CAROL, "USDC", 5_000.0);

    t.advance_and_sync(days(30));
    t.advance_and_sync(days(30));
    t.advance_and_sync(days(30));

    let usdc_rev_before = t.snapshot_revenue("USDC");
    let eth_rev_before = t.snapshot_revenue("ETH");

    t.repay(ALICE, "ETH", 100.0);
    t.repay(BOB, "ETH", 100.0);
    t.repay(CAROL, "USDC", 100_000.0);

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

    // Eve is a pure supplier, so her exit is fully determined: the wallet must
    // move by exactly her credited balance. Counting successes alone cannot see
    // a bug that under-pays every withdrawer and strands the surplus.
    let eve_credited = t.supply_balance_raw(EVE, "USDC");
    let eve_wallet_before = t.token_balance_raw(EVE, "USDC");

    let mut withdraw_successes = 0u32;
    for user in &suppliers {
        if t.try_withdraw(user, "USDC", 999_999.0).is_ok() {
            withdraw_successes += 1;
        }
        if t.try_withdraw(user, "ETH", 999_999.0).is_ok() {
            withdraw_successes += 1;
        }
    }

    assert!(
        withdraw_successes >= 5,
        "all suppliers should successfully withdraw: got {} successes out of 5 suppliers",
        withdraw_successes
    );
    // Exact to the stroop, and directional: withdraw floors in the protocol's
    // favour (ADR-0003), so the payout is the credited balance or one stroop
    // under it -- never over, and never 5% under.
    let eve_paid = t.token_balance_raw(EVE, "USDC") - eve_wallet_before;
    assert!(
        eve_paid == eve_credited || eve_paid == eve_credited - 1,
        "eve's payout must equal her credited supply within the protocol-favouring \
         floor: paid={eve_paid}, credited={eve_credited}"
    );

    // `pool_reserves` is `state.cash`, which the builder pre-loads with
    // `initial_liquidity` (src/multi_hub.rs:80-95). `>= 0.0` therefore had a
    // million-unit margin. Every supplier has exited, so the only cash that may
    // remain is that donation plus the unclaimed protocol revenue.
    let usdc_reserves = t.pool_reserves("USDC");
    let eth_reserves = t.pool_reserves("ETH");
    assert!(
        usdc_reserves >= usdc_preset().initial_liquidity,
        "USDC pool leaked below its seeded liquidity: reserves={}, seeded={}",
        usdc_reserves,
        usdc_preset().initial_liquidity
    );
    assert!(
        eth_reserves >= eth_preset().initial_liquidity,
        "ETH pool leaked below its seeded liquidity: reserves={}, seeded={}",
        eth_reserves,
        eth_preset().initial_liquidity
    );

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

#[test]
fn test_chaos_sustained_high_utilization() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);

    t.borrow(BOB, "USDC", 85_000.0);

    let mut prev_debt = t.borrow_balance(BOB, "USDC");
    let mut prev_supply = t.supply_balance(ALICE, "USDC");

    for month in 1..=12 {
        t.advance_and_sync(days(30));

        let new_debt = t.borrow_balance(BOB, "USDC");
        let new_supply = t.supply_balance(ALICE, "USDC");

        assert!(
            new_debt > prev_debt,
            "month {}: debt should increase: {} -> {}",
            month,
            prev_debt,
            new_debt
        );

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

    let final_debt = t.borrow_balance(BOB, "USDC");
    let growth = final_debt / 85_000.0;
    assert!(
        growth > 1.05,
        "1 year at high utilization should grow debt >5%, actual growth: {:.2}x",
        growth
    );

    let final_hf = t.health_factor(BOB);
    if final_hf < 1.0 {
        assert!(
            t.can_be_liquidated(BOB),
            "low HF account should be liquidatable"
        );
    }
}

#[test]
fn test_chaos_price_oscillation_no_wrongful_liquidation() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    let prices = [1500, 2500, 1800, 2200, 2000];
    for price in &prices {
        t.set_price("ETH", usd(*price));
        t.advance_and_sync(days(1));

        assert!(
            !t.can_be_liquidated(ALICE),
            "well-collateralized account should never be liquidatable at ETH=${}",
            price
        );

        let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
        assert!(
            result.is_err(),
            "liquidation should fail on healthy account at ETH=${}",
            price
        );
    }
}

#[test]
fn test_chaos_multi_market_accounting() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    t.supply(ALICE, "USDC", 200_000.0);
    t.supply(ALICE, "ETH", 10.0);
    t.supply(ALICE, "WBTC", 0.5);

    t.borrow(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "WBTC", 0.01);

    let total_collateral_before = t.total_collateral(ALICE);
    let total_debt_before = t.total_debt(ALICE);
    let hf_before = t.health_factor(ALICE);

    t.advance_and_sync(days(180));

    let total_collateral_after = t.total_collateral(ALICE);
    let total_debt_after = t.total_debt(ALICE);
    let hf_after = t.health_factor(ALICE);

    assert!(
        total_collateral_after >= total_collateral_before,
        "collateral should not decrease: {} -> {}",
        total_collateral_before,
        total_collateral_after
    );

    assert!(
        total_debt_after > total_debt_before,
        "debt should grow with interest: {} -> {}",
        total_debt_before,
        total_debt_after
    );

    assert!(
        hf_after < hf_before,
        "HF should decrease as debt grows: {} -> {}",
        hf_before,
        hf_after
    );

    t.assert_healthy(ALICE);

    t.repay(ALICE, "USDC", 999_999.0);
    t.repay(ALICE, "ETH", 999.0);
    t.repay(ALICE, "WBTC", 999.0);

    let final_debt = t.total_debt(ALICE);
    assert!(
        final_debt < 1.0,
        "debt should be ~0 after full repay, got {}",
        final_debt
    );
}

#[test]
fn test_chaos_keeper_revenue_lifecycle() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 50.0);
    t.borrow(ALICE, "ETH", 10.0);
    t.borrow(BOB, "USDC", 30_000.0);

    t.advance_time(days(7));
    t.update_indexes_for(&["USDC", "ETH"]);

    let usdc_addr = t.resolve_asset("USDC");
    let eth_addr = t.resolve_asset("ETH");
    let ctrl = t.ctrl_client();
    let usdc_assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(usdc_addr)]);
    let eth_assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(eth_addr)]);
    let usdc_index = ctrl
        .get_market_indexes_detailed(&usdc_assets)
        .get(0)
        .unwrap();
    let eth_index = ctrl
        .get_market_indexes_detailed(&eth_assets)
        .get(0)
        .unwrap();
    assert!(
        usdc_index.borrow_index > RAY,
        "USDC borrow index should increase"
    );
    assert!(
        eth_index.borrow_index > RAY,
        "ETH borrow index should increase"
    );

    t.advance_time(days(30));
    t.update_indexes_for(&["USDC", "ETH"]);

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

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.ctrl_client().set_accumulator(&accumulator);

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

    t.advance_and_sync(days(60));

    t.repay(ALICE, "ETH", 999.0);
    t.repay(BOB, "USDC", 999_999.0);

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

    assert!(t.pool_reserves("USDC") >= 0.0, "USDC pool solvent");
    assert!(t.pool_reserves("ETH") >= 0.0, "ETH pool solvent");
}
