use controller::constants::RAY;
use test_harness::{
    days, eth_preset, hub_asset, usdc_preset, wbtc_preset, HubAssetKey, LendingTest, ALICE, BOB,
    CAROL,
};

fn get_indexes(t: &LendingTest, asset: &str) -> (i128, i128) {
    let asset_addr = t.resolve_asset(asset);
    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset_addr)]);
    let idx = ctrl.get_market_indexes_detailed(&assets).get(0).unwrap();
    (idx.supply_index, idx.borrow_index)
}

#[test]
fn test_borrow_index_matches_compound_formula() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.borrow(ALICE, "ETH", 10.0);

    let (_si_before, bi_before) = get_indexes(&t, "ETH");
    assert_eq!(bi_before, RAY, "fresh borrow index should be 1.0 RAY");

    t.advance_and_sync(days(365));

    let (_si_after, bi_after) = get_indexes(&t, "ETH");

    let growth = bi_after as f64 / RAY as f64;
    assert!(
        growth > 1.01 && growth < 1.05,
        "borrow index should grow ~1.8% at 10% utilization, got {:.6}x",
        growth
    );

    assert!(bi_after > bi_before, "borrow index must increase");
}

#[test]
fn test_supply_index_reflects_interest_minus_reserve_factor() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 500_000.0);
    t.borrow(ALICE, "ETH", 50.0);

    let (si_before, bi_before) = get_indexes(&t, "ETH");

    t.advance_and_sync(days(365));

    let (si_after, bi_after) = get_indexes(&t, "ETH");

    let bi_growth = bi_after as f64 / bi_before as f64;
    let si_growth = si_after as f64 / si_before as f64;

    assert!(
        si_growth > 1.0,
        "supply index should increase: {:.6}",
        si_growth
    );
    assert!(
        bi_growth > 1.0,
        "borrow index should increase: {:.6}",
        bi_growth
    );

    assert!(
        si_growth < bi_growth,
        "supply index growth ({:.6}) should be less than borrow index growth ({:.6})",
        si_growth,
        bi_growth
    );
}

#[test]
fn test_interest_accounting_identity() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "ETH", 100.0);
    t.supply(BOB, "USDC", 500_000.0);
    t.borrow(BOB, "ETH", 30.0);

    let supply_before = t.supply_balance(ALICE, "ETH");
    let debt_before = t.borrow_balance(BOB, "ETH");
    let rev_before = t.snapshot_revenue("ETH");

    t.advance_and_sync(days(365));

    let supply_after = t.supply_balance(ALICE, "ETH");
    let debt_after = t.borrow_balance(BOB, "ETH");
    let rev_after = t.snapshot_revenue("ETH");

    let borrower_interest = debt_after - debt_before;
    let supplier_interest = supply_after - supply_before;

    let protocol_revenue_raw = rev_after - rev_before;
    let protocol_revenue = protocol_revenue_raw as f64 / 1e7;

    let total_earned = supplier_interest + protocol_revenue;
    let ratio = if borrower_interest > 0.0 {
        total_earned / borrower_interest
    } else {
        1.0
    };

    assert!(
        borrower_interest > 0.001,
        "borrower should pay meaningful interest: {}",
        borrower_interest
    );
    assert!(
        supplier_interest > 0.0,
        "supplier should earn interest: {}",
        supplier_interest
    );
    assert!(
        protocol_revenue > 0.0,
        "protocol should earn revenue: {}",
        protocol_revenue
    );
    assert!(
        (ratio - 1.0).abs() < 0.02,
        "accounting identity violated: borrower_interest({:.6}) != supplier_interest({:.6}) + protocol_revenue({:.6}), ratio={:.4}",
        borrower_interest, supplier_interest, protocol_revenue, ratio
    );
}

#[test]
fn test_reserve_factor_exact_split() {
    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .build();

    t.supply(ALICE, "ETH", 100.0);
    t.supply(BOB, "USDC", 500_000.0);
    t.borrow(BOB, "ETH", 50.0);

    let supply_before = t.supply_balance(ALICE, "ETH");
    let debt_before = t.borrow_balance(BOB, "ETH");
    let rev_before_raw = t.snapshot_revenue("ETH");

    t.advance_and_sync(days(365));

    let supply_after = t.supply_balance(ALICE, "ETH");
    let debt_after = t.borrow_balance(BOB, "ETH");
    let rev_after_raw = t.snapshot_revenue("ETH");

    let borrower_interest = debt_after - debt_before;
    let supplier_interest = supply_after - supply_before;

    let protocol_revenue = (rev_after_raw - rev_before_raw) as f64 / 1e7;

    let protocol_share = protocol_revenue / borrower_interest;
    assert!(
        (protocol_share - 0.10).abs() < 0.02,
        "protocol should get ~10% of interest (reserve_factor=1000 BPS), got {:.4} ({:.2}%)",
        protocol_share,
        protocol_share * 100.0
    );

    let supplier_share = supplier_interest / borrower_interest;
    assert!(
        (supplier_share - 0.90).abs() < 0.02,
        "suppliers should get ~90% of interest, got {:.4} ({:.2}%)",
        supplier_share,
        supplier_share * 100.0
    );
}

#[test]
fn test_scaled_amount_times_index_equals_actual() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 50.0);
    t.borrow(ALICE, "ETH", 10.0);

    t.advance_and_sync(days(180));

    let account_id = t.resolve_account_id(ALICE);
    let eth_addr = t.resolve_asset("ETH");

    let actual_borrow = t.borrow_balance_raw(ALICE, "ETH");

    let (_, borrow_index) = get_indexes(&t, "ETH");

    let scaled_borrow = t.env.as_contract(&t.controller_address(), || {
        let map: soroban_sdk::Map<HubAssetKey, controller::types::DebtPositionRaw> = t
            .env
            .storage()
            .persistent()
            .get(&controller::types::ControllerKey::BorrowPositions(
                account_id,
            ))
            .expect("borrow side map must exist");
        map.get(hub_asset(eth_addr.clone()))
            .expect("borrow position for asset must exist")
            .scaled_amount
    });

    let actual_in_ray = (scaled_borrow as f64 * borrow_index as f64) / RAY as f64;

    let computed_actual = actual_in_ray / 10f64.powi(20);
    let reported_actual = actual_borrow as f64;

    let diff = (computed_actual - reported_actual).abs();
    let one_unit = 10f64.powi(7);
    assert!(
        diff < one_unit * 2.0,
        "scaled * index / RAY should equal actual: computed={:.0}, reported={:.0}, diff={:.0}",
        computed_actual,
        reported_actual,
        diff
    );
}

#[test]
fn test_rate_curve_three_regions() {
    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .build();

    t.supply(ALICE, "ETH", 1000.0);
    t.supply(BOB, "USDC", 10_000_000.0);

    t.borrow(BOB, "ETH", 200.0);
    let rate_20pct = t.pool_borrow_rate("ETH");

    assert!(rate_20pct > 0.0, "rate at 20% util should be positive");

    t.borrow(BOB, "ETH", 200.0);
    let rate_40pct = t.pool_borrow_rate("ETH");
    assert!(
        rate_40pct > rate_20pct,
        "40% util rate should exceed 20%: {} > {}",
        rate_40pct,
        rate_20pct
    );

    t.borrow(BOB, "ETH", 200.0);
    let rate_60pct = t.pool_borrow_rate("ETH");

    assert!(
        rate_60pct > rate_40pct,
        "60% util rate (region 2) should exceed 40% (region 1): {} > {}",
        rate_60pct,
        rate_40pct
    );

    let slope_r1 = (rate_40pct - rate_20pct) / 0.20;
    let slope_r2 = (rate_60pct - rate_40pct) / 0.20;
    assert!(
        slope_r2 > slope_r1,
        "region 2 slope should be steeper than region 1: r2={:.6} > r1={:.6}",
        slope_r2,
        slope_r1
    );

    t.borrow(BOB, "ETH", 250.0);
    let rate_85pct = t.pool_borrow_rate("ETH");

    assert!(
        rate_85pct > rate_60pct,
        "85% util rate (region 3) should far exceed 60% (region 2): {} > {}",
        rate_85pct,
        rate_60pct
    );

    let jump = rate_85pct / rate_60pct;
    assert!(
        jump > 3.0,
        "region 3 rate should be >3x region 2: {:.2}x",
        jump
    );
}

#[test]
fn test_single_vs_multi_sync_taylor_accuracy() {
    let mut t_single = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t_single.supply(ALICE, "USDC", 100_000.0);
    t_single.supply(BOB, "ETH", 100.0);
    t_single.borrow(ALICE, "ETH", 10.0);
    t_single.advance_and_sync(days(365));
    let debt_single = t_single.borrow_balance(ALICE, "ETH");

    let mut t_multi = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t_multi.supply(ALICE, "USDC", 100_000.0);
    t_multi.supply(BOB, "ETH", 100.0);
    t_multi.borrow(ALICE, "ETH", 10.0);
    for _ in 0..365 {
        t_multi.advance_and_sync(days(1));
    }
    let debt_multi = t_multi.borrow_balance(ALICE, "ETH");

    let interest_single = debt_single - 10.0;
    let interest_multi = debt_multi - 10.0;

    let diff_pct = if interest_single > 0.0 {
        ((interest_single - interest_multi) / interest_single * 100.0).abs()
    } else {
        0.0
    };

    assert!(
        diff_pct < 5.0,
        "single vs multi sync should differ < 5%: single_interest={:.6}, multi_interest={:.6}, diff={:.2}%",
        interest_single, interest_multi, diff_pct
    );
}

#[test]
fn test_supply_index_unchanged_without_borrows() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let (si_before, bi_before) = get_indexes(&t, "USDC");
    assert_eq!(si_before, RAY, "initial supply index should be 1.0 RAY");
    assert_eq!(bi_before, RAY, "initial borrow index should be 1.0 RAY");

    t.advance_and_sync(days(365));

    let (si_after, _bi_after) = get_indexes(&t, "USDC");

    assert_eq!(
        si_after, RAY,
        "supply index should stay at 1.0 RAY with no borrows, got {}",
        si_after
    );
}

#[test]
fn test_multiple_suppliers_share_proportionally() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "ETH", 75.0);
    t.supply(BOB, "ETH", 25.0);
    t.supply(CAROL, "USDC", 1_000_000.0);
    t.borrow(CAROL, "ETH", 50.0);

    let alice_before = t.supply_balance(ALICE, "ETH");
    let bob_before = t.supply_balance(BOB, "ETH");

    t.advance_and_sync(days(365));

    let alice_after = t.supply_balance(ALICE, "ETH");
    let bob_after = t.supply_balance(BOB, "ETH");

    let alice_interest = alice_after - alice_before;
    let bob_interest = bob_after - bob_before;

    let ratio = alice_interest / bob_interest;
    assert!(
        (ratio - 3.0).abs() < 0.1,
        "Alice (75%) should earn 3x Bob's (25%) interest: ratio={:.4}",
        ratio
    );
}

#[test]
fn test_interest_grows_with_time_checkpoints() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 5.0);

    let mut prev_debt = t.borrow_balance(ALICE, "ETH");
    let mut prev_interest = 0.0f64;

    let checkpoints = [
        (days(1), "1 day"),
        (days(6), "1 week"),
        (days(23), "1 month"),
        (days(60), "3 months"),
        (days(90), "6 months"),
        (days(185), "1 year"),
    ];

    for (advance, label) in &checkpoints {
        t.advance_and_sync(*advance);
        let debt = t.borrow_balance(ALICE, "ETH");
        let interest = debt - 5.0;

        assert!(
            debt > prev_debt,
            "{}: debt should grow: {:.6} > {:.6}",
            label,
            debt,
            prev_debt
        );
        assert!(
            interest > prev_interest,
            "{}: cumulative interest should grow: {:.6} > {:.6}",
            label,
            interest,
            prev_interest
        );

        prev_debt = debt;
        prev_interest = interest;
    }

    assert!(
        prev_interest > 0.01,
        "1 year of interest should be >0.01 ETH, got {:.6}",
        prev_interest
    );
}

#[test]
fn test_pool_solvency_invariant() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "ETH", 100.0);
    t.supply(BOB, "USDC", 500_000.0);
    t.borrow(BOB, "ETH", 50.0);

    for month in 1..=12 {
        t.advance_and_sync(days(30));

        let eth = hub_asset(t.resolve_asset("ETH"));
        let pool_client = t.pool_client("ETH");
        let supplied = pool_client.get_supplied_amount(&eth);
        let borrowed = pool_client.get_borrowed_amount(&eth);
        let revenue = pool_client.get_revenue(&eth);

        assert!(
            supplied >= borrowed,
            "month {}: supplied ({}) must >= borrowed ({})",
            month,
            supplied,
            borrowed
        );

        assert!(
            revenue >= 0,
            "month {}: revenue must be >= 0, got {}",
            month,
            revenue
        );

        assert!(
            revenue <= supplied,
            "month {}: revenue ({}) must <= supplied ({})",
            month,
            revenue,
            supplied
        );
    }
}

#[test]
fn test_index_values_accessible_and_rational() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 100.0);
    t.supply(ALICE, "WBTC", 1.0);
    t.supply(BOB, "USDC", 100_000.0);
    t.borrow(BOB, "ETH", 10.0);
    t.borrow(BOB, "WBTC", 0.5);

    t.advance_and_sync(days(365));

    for asset in &["USDC", "ETH", "WBTC"] {
        let (si, bi) = get_indexes(&t, asset);

        assert!(si >= RAY, "{} supply index {} must be >= RAY", asset, si);
        assert!(bi >= RAY, "{} borrow index {} must be >= RAY", asset, bi);

        assert!(
            bi >= si,
            "{}: borrow index ({}) must be >= supply index ({})",
            asset,
            bi,
            si
        );
    }

    let (_, bi_eth) = get_indexes(&t, "ETH");
    let (_, bi_wbtc) = get_indexes(&t, "WBTC");
    assert!(
        bi_wbtc > bi_eth,
        "higher utilization should produce higher borrow index: WBTC({}) > ETH({})",
        bi_wbtc,
        bi_eth
    );

    let (si_usdc, bi_usdc) = get_indexes(&t, "USDC");
    assert_eq!(
        si_usdc, RAY,
        "USDC supply index should be 1.0 RAY (no borrows)"
    );
    assert!(bi_usdc >= RAY, "USDC borrow index should be >= 1.0 RAY");
}
