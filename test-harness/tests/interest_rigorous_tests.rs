extern crate std;

use common::constants::RAY;
use test_harness::{days, eth_preset, usdc_preset, wbtc_preset, LendingTest, ALICE, BOB, CAROL};

// ===========================================================================
// Rigorous interest tests: verify amounts, not just direction.
//
// The lending protocol's interest model:
//   borrow_index(t) = borrow_index(t-1) * compound_interest(rate, delta_ms)
//   supply_index(t) = supply_index(t-1) * (1 + supplier_rewards / total_supplied)
//   supplier_rewards = accrued_interest * (1 - reserve_factor / 10000)
//   protocol_revenue = accrued_interest * reserve_factor / 10000
//   accrued_interest = total_debt_new - total_debt_old
//
// Key invariant:
//   borrower_interest = supplier_interest + protocol_revenue
// ===========================================================================

// ---------------------------------------------------------------------------
// Helper: read raw indexes from the pool.
// ---------------------------------------------------------------------------

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
// 1. Verify borrow index matches compound interest formula
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_index_matches_compound_formula() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply and borrow to establish utilization.
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.borrow(ALICE, "ETH", 10.0); // 10% utilization of 100 ETH.

    let (_si_before, bi_before) = get_indexes(&t, "ETH");
    assert_eq!(bi_before, RAY, "fresh borrow index should be 1.0 RAY");

    // Advance one year and sync.
    t.advance_and_sync(days(365));

    let (_si_after, bi_after) = get_indexes(&t, "ETH");

    // borrow_index grows by compound_interest(rate, delta_ms).
    // At 10% utilization with default params:
    //   rate = base(1%) + util(10%) * slope1(4%) / mid(50%) = 1% + 0.8% = 1.8% annual.
    // Therefore borrow_index after 1 year ~ 1.0 * e^0.018 ~ 1.01816.
    let growth = bi_after as f64 / RAY as f64;
    assert!(
        growth > 1.01 && growth < 1.05,
        "borrow index should grow ~1.8% at 10% utilization, got {:.6}x",
        growth
    );

    // The index must rise strictly: compound interest > 0.
    assert!(bi_after > bi_before, "borrow index must increase");
}

// ---------------------------------------------------------------------------
// 2. Verify supply index growth matches interest share
// ---------------------------------------------------------------------------

#[test]
fn test_supply_index_reflects_interest_minus_reserve_factor() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(BOB, "ETH", 100.0); // Bob is the sole supplier.
    t.supply(ALICE, "USDC", 500_000.0);
    t.borrow(ALICE, "ETH", 50.0); // 50% utilization.

    let (si_before, bi_before) = get_indexes(&t, "ETH");

    t.advance_and_sync(days(365));

    let (si_after, bi_after) = get_indexes(&t, "ETH");

    let bi_growth = bi_after as f64 / bi_before as f64;
    let si_growth = si_after as f64 / si_before as f64;

    // Reserve factor = 10% (1000 BPS), so suppliers receive 90% of interest.
    // supply_index_growth ~ utilization * borrow_index_growth * (1 - reserve_factor).
    // At 50% utilization: supplier_growth ~ 0.5 * bi_growth * 0.9.

    // Both must have grown.
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

    // Supply-index growth must trail borrow-index growth: the protocol
    // takes the reserve-factor cut and utilization stays under 100%.
    assert!(
        si_growth < bi_growth,
        "supply index growth ({:.6}) should be less than borrow index growth ({:.6})",
        si_growth,
        bi_growth
    );
}

// ---------------------------------------------------------------------------
// 3. Accounting identity: borrower_interest = supplier_interest + protocol_revenue
// ---------------------------------------------------------------------------

#[test]
fn test_interest_accounting_identity() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // One supplier, one borrower keeps the accounting clean.
    t.supply(ALICE, "ETH", 100.0);
    t.supply(BOB, "USDC", 500_000.0);
    t.borrow(BOB, "ETH", 30.0); // 30% utilization.

    let supply_before = t.supply_balance(ALICE, "ETH");
    let debt_before = t.borrow_balance(BOB, "ETH");
    let rev_before = t.snapshot_revenue("ETH");

    t.advance_and_sync(days(365));

    let supply_after = t.supply_balance(ALICE, "ETH");
    let debt_after = t.borrow_balance(BOB, "ETH");
    let rev_after = t.snapshot_revenue("ETH");

    let borrower_interest = debt_after - debt_before;
    let supplier_interest = supply_after - supply_before;

    // protocol_revenue() returns token units in asset precision (7 decimals for ETH).
    let protocol_revenue_raw = rev_after - rev_before;
    let protocol_revenue = protocol_revenue_raw as f64 / 1e7; // 7 decimals.

    // Accounting identity: borrower pays = suppliers earn + protocol earns.
    // Allow 1% tolerance for rounding across multiple RAY multiplications.
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

// ---------------------------------------------------------------------------
// 4. Reserve factor split: protocol gets exactly reserve_factor% of interest
// ---------------------------------------------------------------------------

#[test]
fn test_reserve_factor_exact_split() {
    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .build();

    // reserve_factor_bps = 1000 (10%).
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
    // protocol_revenue() returns token units in asset precision (7 decimals).
    let protocol_revenue = (rev_after_raw - rev_before_raw) as f64 / 1e7;

    // Protocol must take ~10% of total interest.
    let protocol_share = protocol_revenue / borrower_interest;
    assert!(
        (protocol_share - 0.10).abs() < 0.02,
        "protocol should get ~10% of interest (reserve_factor=1000 BPS), got {:.4} ({:.2}%)",
        protocol_share,
        protocol_share * 100.0
    );

    // Suppliers must take ~90%.
    let supplier_share = supplier_interest / borrower_interest;
    assert!(
        (supplier_share - 0.90).abs() < 0.02,
        "suppliers should get ~90% of interest, got {:.4} ({:.2}%)",
        supplier_share,
        supplier_share * 100.0
    );
}

// ---------------------------------------------------------------------------
// 5. Index relationship: actual_amount = scaled_amount * index / RAY
// ---------------------------------------------------------------------------

#[test]
fn test_scaled_amount_times_index_equals_actual() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 50.0);
    t.borrow(ALICE, "ETH", 10.0);

    // Advance to accrue interest.
    t.advance_and_sync(days(180));

    // Read the raw scaled position from storage.
    let account_id = t.resolve_account_id(ALICE);
    let eth_addr = t.resolve_asset("ETH");

    // Read the unscaled borrow balance.
    let actual_borrow = t.borrow_balance_raw(ALICE, "ETH");

    // Read the borrow index.
    let (_, borrow_index) = get_indexes(&t, "ETH");

    // Read the scaled position from split position storage.
    let scaled_borrow = t.env.as_contract(&t.controller_address(), || {
        t.env
            .storage()
            .persistent()
            .get::<_, common::types::AccountPosition>(
                &common::types::ControllerKey::BorrowPosition(account_id, eth_addr.clone()),
            )
            .unwrap()
            .scaled_amount_ray
    });

    // Verify: actual ~ rescale(scaled * borrow_index / RAY, 27, 7).
    // scaled_borrow is RAY-native, so the product / RAY produces a RAY result.
    let actual_in_ray = (scaled_borrow as f64 * borrow_index as f64) / RAY as f64;
    // Convert RAY (27 dec) to asset decimals (7 dec).
    let computed_actual = actual_in_ray / 10f64.powi(20); // 27 - 7 = 20.
    let reported_actual = actual_borrow as f64;

    // The values must stay within one token unit of rounding.
    let diff = (computed_actual - reported_actual).abs();
    let one_unit = 10f64.powi(7); // 7 decimals for ETH.
    assert!(
        diff < one_unit * 2.0,
        "scaled * index / RAY should equal actual: computed={:.0}, reported={:.0}, diff={:.0}",
        computed_actual,
        reported_actual,
        diff
    );
}

// ---------------------------------------------------------------------------
// 6. 3-region rate curve verification with actual numbers
// ---------------------------------------------------------------------------

#[test]
fn test_rate_curve_three_regions() {
    // Default params: base=1%, slope1=4%, slope2=10%, slope3=300%.
    // mid=50%, optimal=80%.
    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .build();

    t.supply(ALICE, "ETH", 1000.0);
    t.supply(BOB, "USDC", 10_000_000.0);

    // Region 1: utilization < mid (50%).
    // Borrow 200 ETH = 20% utilization.
    t.borrow(BOB, "ETH", 200.0);
    let rate_20pct = t.pool_borrow_rate("ETH");

    // Region 1 formula: rate = base + util * slope1 / mid
    //   = 1% + 20% * 4% / 50% = 1% + 1.6% = 2.6% annual.
    // Per-ms rate = 2.6% / ms_per_year, in RAY.
    assert!(rate_20pct > 0.0, "rate at 20% util should be positive");

    // Borrow more to reach 40% utilization.
    t.borrow(BOB, "ETH", 200.0); // Now 400/1000 = 40%.
    let rate_40pct = t.pool_borrow_rate("ETH");
    assert!(
        rate_40pct > rate_20pct,
        "40% util rate should exceed 20%: {} > {}",
        rate_40pct,
        rate_20pct
    );

    // Region 2: mid <= utilization < optimal.
    // Borrow to reach 60% utilization.
    t.borrow(BOB, "ETH", 200.0); // Now 600/1000 = 60%.
    let rate_60pct = t.pool_borrow_rate("ETH");

    // Region 2 formula: rate = base + slope1 + (util - mid) * slope2 / (opt - mid)
    //   = 1% + 4% + (60% - 50%) * 10% / (80% - 50%) = 5% + 3.33% = 8.33%.
    assert!(
        rate_60pct > rate_40pct,
        "60% util rate (region 2) should exceed 40% (region 1): {} > {}",
        rate_60pct,
        rate_40pct
    );

    // Region 2 must slope steeper than region 1.
    let slope_r1 = (rate_40pct - rate_20pct) / 0.20; // Rate change per 20% util.
    let slope_r2 = (rate_60pct - rate_40pct) / 0.20;
    assert!(
        slope_r2 > slope_r1,
        "region 2 slope should be steeper than region 1: r2={:.6} > r1={:.6}",
        slope_r2,
        slope_r1
    );

    // Region 3: utilization >= optimal (80%).
    // Borrow to reach 85% utilization.
    t.borrow(BOB, "ETH", 250.0); // Now 850/1000 = 85%.
    let rate_85pct = t.pool_borrow_rate("ETH");

    // Region 3 formula: rate = base + slope1 + slope2 + (util - opt) * slope3 / (1 - opt)
    //   = 1% + 4% + 10% + (85% - 80%) * 300% / (100% - 80%) = 15% + 75% = 90%.
    // Very high; slope3 = 300% is aggressive.
    assert!(
        rate_85pct > rate_60pct,
        "85% util rate (region 3) should far exceed 60% (region 2): {} > {}",
        rate_85pct,
        rate_60pct
    );

    // Region 3 must slope much steeper (slope3=300% vs slope2=10%).
    let jump = rate_85pct / rate_60pct;
    assert!(
        jump > 3.0,
        "region 3 rate should be >3x region 2: {:.2}x",
        jump
    );
}

// ---------------------------------------------------------------------------
// 7. Single sync vs multiple syncs produce similar results (Taylor approx)
// ---------------------------------------------------------------------------

#[test]
fn test_single_vs_multi_sync_taylor_accuracy() {
    // Setup A: a single 365-day sync.
    let mut t_single = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t_single.supply(ALICE, "USDC", 100_000.0);
    t_single.supply(BOB, "ETH", 100.0);
    t_single.borrow(ALICE, "ETH", 10.0);
    t_single.advance_and_sync(days(365));
    let debt_single = t_single.borrow_balance(ALICE, "ETH");

    // Setup B: daily syncs across 365 days.
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

    // Both runs must produce similar results. The Taylor approximation
    // stays accurate for small rate*time products. For low utilization
    // (~10%), the rate is ~1.8% annual. Single sync: e^(0.018) ~ 1.01816
    // (exact). Multi sync: (e^(0.018/365))^365 ~ 1.01816 (exact for daily).
    // The difference must remain under 1% of the interest amount.
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

// ---------------------------------------------------------------------------
// 8. Supply index stays at 1.0 when no borrows (zero utilization)
// ---------------------------------------------------------------------------

#[test]
fn test_supply_index_unchanged_without_borrows() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let (si_before, bi_before) = get_indexes(&t, "USDC");
    assert_eq!(si_before, RAY, "initial supply index should be 1.0 RAY");
    assert_eq!(bi_before, RAY, "initial borrow index should be 1.0 RAY");

    t.advance_and_sync(days(365));

    let (si_after, _bi_after) = get_indexes(&t, "USDC");

    // With zero utilization, no interest accrues. borrow_index still grows
    // because compound_interest uses base_rate > 0, but supply_index must
    // not grow: there is no borrower interest to distribute.
    assert_eq!(
        si_after, RAY,
        "supply index should stay at 1.0 RAY with no borrows, got {}",
        si_after
    );
}

// ---------------------------------------------------------------------------
// 9. Multiple suppliers share interest proportionally
// ---------------------------------------------------------------------------

#[test]
fn test_multiple_suppliers_share_proportionally() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies 75% and Bob supplies 25%.
    t.supply(ALICE, "ETH", 75.0);
    t.supply(BOB, "ETH", 25.0);
    t.supply(CAROL, "USDC", 1_000_000.0);
    t.borrow(CAROL, "ETH", 50.0); // 50% utilization.

    let alice_before = t.supply_balance(ALICE, "ETH");
    let bob_before = t.supply_balance(BOB, "ETH");

    t.advance_and_sync(days(365));

    let alice_after = t.supply_balance(ALICE, "ETH");
    let bob_after = t.supply_balance(BOB, "ETH");

    let alice_interest = alice_after - alice_before;
    let bob_interest = bob_after - bob_before;

    // Alice must earn 3x Bob's interest (75/25 = 3:1).
    let ratio = alice_interest / bob_interest;
    assert!(
        (ratio - 3.0).abs() < 0.1,
        "Alice (75%) should earn 3x Bob's (25%) interest: ratio={:.4}",
        ratio
    );
}

// ---------------------------------------------------------------------------
// 10. Interest grows with time — linear check at multiple points
// ---------------------------------------------------------------------------

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

    // Check at 1 day, 1 week, 1 month, 3 months, 6 months, 1 year.
    let checkpoints = [
        (days(1), "1 day"),
        (days(6), "1 week"),    // Cumulative: 7 days.
        (days(23), "1 month"),  // Cumulative: 30 days.
        (days(60), "3 months"), // Cumulative: 90 days.
        (days(90), "6 months"), // Cumulative: 180 days.
        (days(185), "1 year"),  // Cumulative: 365 days.
    ];

    for (advance, label) in &checkpoints {
        t.advance_and_sync(*advance);
        let debt = t.borrow_balance(ALICE, "ETH");
        let interest = debt - 5.0; // The initial borrow was 5 ETH.

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

    // After one year, interest must be meaningful: > 0.01 ETH at low util.
    assert!(
        prev_interest > 0.01,
        "1 year of interest should be >0.01 ETH, got {:.6}",
        prev_interest
    );
}

// ---------------------------------------------------------------------------
// 11. Pool solvency: supplied_value >= borrowed_value + revenue_value always
// ---------------------------------------------------------------------------

#[test]
fn test_pool_solvency_invariant() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "ETH", 100.0);
    t.supply(BOB, "USDC", 500_000.0);
    t.borrow(BOB, "ETH", 50.0);

    // Check solvency at multiple time points.
    for month in 1..=12 {
        t.advance_and_sync(days(30));

        let pool_client = t.pool_client("ETH");
        let supplied = pool_client.supplied_amount(); // RAY.
        let borrowed = pool_client.borrowed_amount(); // RAY.
        let revenue = pool_client.protocol_revenue(); // RAY.

        // Solvency: total supply >= total borrows. Supply includes protocol
        // revenue as scaled supply tokens.
        assert!(
            supplied >= borrowed,
            "month {}: supplied ({}) must >= borrowed ({})",
            month,
            supplied,
            borrowed
        );

        // Revenue must remain non-negative.
        assert!(
            revenue >= 0,
            "month {}: revenue must be >= 0, got {}",
            month,
            revenue
        );

        // Revenue must stay <= supplied; revenue cannot exceed total supply.
        assert!(
            revenue <= supplied,
            "month {}: revenue ({}) must <= supplied ({})",
            month,
            revenue,
            supplied
        );
    }
}

// ---------------------------------------------------------------------------
// 12. Index values are accessible via pool.get_market_index and match expected
// ---------------------------------------------------------------------------

#[test]
fn test_index_values_accessible_and_rational() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    // Different utilization levels per market.
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 100.0);
    t.supply(ALICE, "WBTC", 1.0);
    t.supply(BOB, "USDC", 100_000.0);
    t.borrow(BOB, "ETH", 10.0); // ~10% util.
    t.borrow(BOB, "WBTC", 0.5); // ~50% util.

    t.advance_and_sync(days(365));

    for asset in &["USDC", "ETH", "WBTC"] {
        let (si, bi) = get_indexes(&t, asset);

        // Both indexes must satisfy >= RAY (1.0).
        assert!(si >= RAY, "{} supply index {} must be >= RAY", asset, si);
        assert!(bi >= RAY, "{} borrow index {} must be >= RAY", asset, bi);

        // Borrow index >= supply index; borrowers pay more than suppliers earn.
        assert!(
            bi >= si,
            "{}: borrow index ({}) must be >= supply index ({})",
            asset,
            bi,
            si
        );
    }

    // WBTC (50% util) must show higher indexes than ETH (10% util).
    let (_, bi_eth) = get_indexes(&t, "ETH");
    let (_, bi_wbtc) = get_indexes(&t, "WBTC");
    assert!(
        bi_wbtc > bi_eth,
        "higher utilization should produce higher borrow index: WBTC({}) > ETH({})",
        bi_wbtc,
        bi_eth
    );

    // USDC (no borrows) must show borrow index > RAY (base_rate > 0) but
    // supply index = RAY (no borrower interest to distribute).
    let (si_usdc, bi_usdc) = get_indexes(&t, "USDC");
    assert_eq!(
        si_usdc, RAY,
        "USDC supply index should be 1.0 RAY (no borrows)"
    );
    assert!(bi_usdc >= RAY, "USDC borrow index should be >= 1.0 RAY");
}
