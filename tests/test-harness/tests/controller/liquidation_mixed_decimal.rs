use common::types::SeizeMode;
use test_harness::presets::{
    MarketPreset, ALICE, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, LIQUIDATOR,
};
use test_harness::{helpers::usd, hub_asset, LendingTest};

fn make_market(name: &'static str, decimals: u32, price: i128, liquidity: f64) -> MarketPreset {
    MarketPreset {
        name,
        decimals,
        price_wad: price,
        initial_liquidity: liquidity,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn usdc_6() -> MarketPreset {
    MarketPreset {
        name: "USDC6",
        decimals: 6,
        price_wad: usd(1),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn dai_18() -> MarketPreset {
    MarketPreset {
        name: "DAI18",
        decimals: 18,
        price_wad: usd(1),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn wbtc_8() -> MarketPreset {
    MarketPreset {
        name: "WBTC8",
        decimals: 8,
        price_wad: usd(60_000),
        initial_liquidity: 100_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn sol_9() -> MarketPreset {
    MarketPreset {
        name: "SOL9",
        decimals: 9,
        price_wad: usd(150),
        initial_liquidity: 100_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

#[test]
fn test_liquidation_two_collaterals_6dec_18dec_debt_8dec() {
    let mut t = LendingTest::new()
        .with_market(usdc_6())
        .with_market(dai_18())
        .with_market(wbtc_8())
        .build();

    t.supply(ALICE, "USDC6", 5_000.0);
    t.supply_to(ALICE, t.resolve_account_id(ALICE), "DAI18", 5_000.0);

    t.borrow(ALICE, "WBTC8", 0.125);

    t.set_price("WBTC8", usd(70_000));
    t.advance_and_sync(1000);

    let hf_before = t.health_factor(ALICE);
    assert!(hf_before < 1.0, "HF should be < 1.0, got {}", hf_before);

    let usdc_before = t.supply_balance(ALICE, "USDC6");
    let dai_before = t.supply_balance(ALICE, "DAI18");

    t.liquidate(LIQUIDATOR, ALICE, "WBTC8", 0.03);

    let usdc_after = t.supply_balance(ALICE, "USDC6");
    let dai_after = t.supply_balance(ALICE, "DAI18");

    let usdc_seized = usdc_before - usdc_after;
    let dai_seized = dai_before - dai_after;

    assert!(
        usdc_seized > 0.0,
        "6-dec USDC should have been seized, got seized={}",
        usdc_seized
    );
    assert!(
        dai_seized > 0.0,
        "18-dec DAI should have been seized, got seized={}",
        dai_seized
    );

    let ratio = if usdc_seized > dai_seized {
        usdc_seized / dai_seized
    } else {
        dai_seized / usdc_seized
    };
    assert!(
        ratio < 1.5,
        "Seizure should be roughly proportional across decimals. USDC6 seized={}, DAI18 seized={}, ratio={}",
        usdc_seized, dai_seized, ratio
    );

    let debt_after = t.borrow_balance(ALICE, "WBTC8");
    assert!(
        debt_after < 0.125,
        "Debt should be reduced after liquidation, got {}",
        debt_after
    );
}

#[test]
fn test_liquidation_asymmetric_90pct_6dec_10pct_18dec() {
    let mut t = LendingTest::new()
        .with_market(usdc_6())
        .with_market(dai_18())
        .with_market(sol_9())
        .build();

    t.supply(ALICE, "USDC6", 9_000.0);
    t.supply_to(ALICE, t.resolve_account_id(ALICE), "DAI18", 1_000.0);

    t.borrow(ALICE, "SOL9", 50.0);

    t.set_price("SOL9", usd(175));
    t.advance_and_sync(1000);

    assert!(t.health_factor(ALICE) < 1.0, "Should be liquidatable");

    let dai_before = t.supply_balance(ALICE, "DAI18");
    let usdc_before = t.supply_balance(ALICE, "USDC6");

    t.liquidate(LIQUIDATOR, ALICE, "SOL9", 10.0);

    let dai_after = t.supply_balance(ALICE, "DAI18");
    let usdc_after = t.supply_balance(ALICE, "USDC6");

    let dai_seized = dai_before - dai_after;
    let usdc_seized = usdc_before - usdc_after;

    assert!(
        dai_seized > 0.0,
        "Even the 10% DAI18 position should be partially seized, got seized={}",
        dai_seized
    );
    assert!(
        usdc_seized > 0.0,
        "USDC6 (90%) should be seized, got seized={}",
        usdc_seized
    );

    let ratio = usdc_seized / dai_seized;
    assert!(
        ratio > 5.0 && ratio < 15.0,
        "USDC seizure should be ~9x DAI seizure. USDC={}, DAI={}, ratio={}",
        usdc_seized,
        dai_seized,
        ratio
    );
}

#[test]
fn test_liquidation_multi_debt_6dec_and_18dec() {
    let mut t = LendingTest::new()
        .with_market(usdc_6())
        .with_market(dai_18())
        .with_market(sol_9())
        .build();

    t.supply(ALICE, "SOL9", 133.0);

    t.borrow(ALICE, "USDC6", 7_000.0);
    t.borrow(ALICE, "DAI18", 7_000.0);

    t.set_price("SOL9", usd(120));
    t.advance_and_sync(1000);

    assert!(t.health_factor(ALICE) < 1.0, "Should be liquidatable");

    let usdc_debt_before = t.borrow_balance(ALICE, "USDC6");
    let dai_debt_before = t.borrow_balance(ALICE, "DAI18");

    t.liquidate_multi(LIQUIDATOR, ALICE, &[("USDC6", 2_000.0), ("DAI18", 2_000.0)]);

    let usdc_debt_after = t.borrow_balance(ALICE, "USDC6");
    let dai_debt_after = t.borrow_balance(ALICE, "DAI18");

    assert!(
        usdc_debt_after < usdc_debt_before,
        "USDC6 debt should decrease: before={}, after={}",
        usdc_debt_before,
        usdc_debt_after
    );
    assert!(
        dai_debt_after < dai_debt_before,
        "DAI18 debt should decrease: before={}, after={}",
        dai_debt_before,
        dai_debt_after
    );

    let sol_after = t.supply_balance(ALICE, "SOL9");
    assert!(
        sol_after < 133.0,
        "SOL9 collateral should be seized, remaining={}",
        sol_after
    );
}

#[test]
fn test_liquidation_multi_debt_different_decimals() {
    let mut t = LendingTest::new()
        .with_market(usdc_6())
        .with_market(dai_18())
        .with_market(sol_9())
        .build();

    t.supply(ALICE, "DAI18", 20_000.0);

    t.borrow(ALICE, "USDC6", 7_000.0);
    t.borrow(ALICE, "SOL9", 46.7);

    t.set_price("DAI18", usd(1) * 85 / 100);
    t.advance_and_sync(1000);

    assert!(t.health_factor(ALICE) < 1.0, "Should be liquidatable");

    t.liquidate_multi(LIQUIDATOR, ALICE, &[("USDC6", 2_000.0), ("SOL9", 10.0)]);

    assert!(
        t.borrow_balance(ALICE, "USDC6") < 7_000.0,
        "USDC6 debt reduced"
    );
    assert!(t.borrow_balance(ALICE, "SOL9") < 46.7, "SOL9 debt reduced");

    assert!(
        t.supply_balance(ALICE, "DAI18") < 20_000.0,
        "DAI18 collateral seized"
    );
}

#[test]
fn test_bad_debt_cleanup_mixed_decimals() {
    let mut t = LendingTest::new()
        .with_market(usdc_6())
        .with_market(dai_18())
        .build();

    t.supply(ALICE, "USDC6", 200.0);

    t.borrow(ALICE, "DAI18", 150.0);

    t.set_price("USDC6", usd(1) / 1000);
    t.advance_and_sync(1000);

    let hf = t.health_factor(ALICE);
    assert!(hf < 0.01, "HF should be deeply underwater, got {}", hf);

    t.liquidate(LIQUIDATOR, ALICE, "DAI18", 10.0);

    // Sub-threshold collateral against real debt: bad-debt cleanup must fire,
    // clearing every position and removing the account entry.
    t.assert_no_positions(ALICE);
    assert_eq!(
        t.get_active_accounts(ALICE).len(),
        0,
        "bad-debt cleanup must remove the account across mixed decimals"
    );
}

#[test]
fn test_liquidation_protocol_fee_cross_decimal() {
    let mut t = LendingTest::new()
        .with_market(usdc_6())
        .with_market(dai_18())
        .build();

    t.supply(ALICE, "USDC6", 10_000.0);
    t.borrow(ALICE, "DAI18", 7_500.0);

    t.set_price("USDC6", usd(1) * 90 / 100);
    t.advance_and_sync(1000);

    assert!(t.health_factor(ALICE) < 1.0);

    let collateral_before = t.total_collateral(ALICE);

    // The named property: the fee is charged on the bonus leg only, and lands
    // in the 6-decimal collateral market while the debt leg is 18-decimal.
    // Without this the body never read a fee at all, so 0x or 1e12x passed.
    let account_id = t.resolve_account_id(ALICE);
    let payments = soroban_sdk::Vec::from_array(
        &t.env,
        [(hub_asset(t.resolve_asset("DAI18")), 2_000 * 10i128.pow(18))],
    );
    let estimate =
        t.ctrl_client()
            .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Transfer);
    let seized = estimate.seized_collaterals.get_unchecked(0).amount;
    let fee = estimate.protocol_fees.get_unchecked(0).amount;
    let bonus_bps = estimate.bonus_rate_bps;
    let fee_bps = i128::from(t.get_asset_config("USDC6").liquidation_fees);
    assert!(
        seized > 0 && fee > 0 && bonus_bps > 0 && fee_bps > 0,
        "estimate must be live: seized={seized}, fee={fee}, bonus_bps={bonus_bps}"
    );
    // seized = principal * (1 + b), so the bonus portion is seized * b / (1 + b).
    let expected_fee = (seized * bonus_bps / (10_000 + bonus_bps)) * fee_bps / 10_000;
    assert!(
        (fee - expected_fee).abs() <= 1,
        "cross-decimal fee {fee} must equal the bonus-only charge {expected_fee} \
         (seized={seized}, bonus_bps={bonus_bps}, fee_bps={fee_bps})"
    );

    let revenue_before = t.snapshot_revenue("USDC6");

    t.liquidate(LIQUIDATOR, ALICE, "DAI18", 2_000.0);

    let revenue_delta = t.snapshot_revenue("USDC6") - revenue_before;
    assert!(
        (revenue_delta - fee).abs() <= 2,
        "the 6-decimal market must book the fee as revenue: delta={revenue_delta}, fee={fee}"
    );

    let collateral_after = t.total_collateral(ALICE);
    let debt_after = t.total_debt(ALICE);

    assert!(
        collateral_after < collateral_before,
        "Collateral should decrease: before={}, after={}",
        collateral_before,
        collateral_after
    );

    assert!(
        debt_after < 7_500.0,
        "Debt should decrease, got {}",
        debt_after
    );
}

#[test]
fn test_liquidation_2x2_four_unique_decimals() {
    let mut t = LendingTest::new()
        .with_market(make_market("C6", 6, usd(1), 1_000_000.0))
        .with_market(make_market("C18", 18, usd(1), 1_000_000.0))
        .with_market(make_market("D8", 8, usd(60_000), 100_000.0))
        .with_market(make_market("D9", 9, usd(150), 100_000.0))
        .build();

    t.supply(ALICE, "C6", 5_000.0);
    let acct = t.resolve_account_id(ALICE);
    t.supply_to(ALICE, acct, "C18", 5_000.0);

    t.borrow(ALICE, "D8", 0.058);
    t.borrow(ALICE, "D9", 23.0);

    t.assert_healthy(ALICE);

    t.set_price("C6", usd(1) * 85 / 100);
    t.set_price("C18", usd(1) * 85 / 100);
    t.advance_and_sync(1000);

    assert!(t.health_factor(ALICE) < 1.0, "Should be liquidatable");

    let c6_before = t.supply_balance(ALICE, "C6");
    let c18_before = t.supply_balance(ALICE, "C18");
    let d8_before = t.borrow_balance(ALICE, "D8");
    let d9_before = t.borrow_balance(ALICE, "D9");

    t.liquidate_multi(LIQUIDATOR, ALICE, &[("D8", 0.01), ("D9", 5.0)]);

    let d8_after = t.borrow_balance(ALICE, "D8");
    let d9_after = t.borrow_balance(ALICE, "D9");
    assert!(
        d8_after < d8_before,
        "D8 (8-dec) debt: {} -> {}",
        d8_before,
        d8_after
    );
    assert!(
        d9_after < d9_before,
        "D9 (9-dec) debt: {} -> {}",
        d9_before,
        d9_after
    );

    let c6_after = t.supply_balance(ALICE, "C6");
    let c18_after = t.supply_balance(ALICE, "C18");
    let c6_seized = c6_before - c6_after;
    let c18_seized = c18_before - c18_after;

    assert!(
        c6_seized > 0.0,
        "C6 (6-dec) must be seized, got {}",
        c6_seized
    );
    assert!(
        c18_seized > 0.0,
        "C18 (18-dec) must be seized, got {}",
        c18_seized
    );

    let c6_usd = c6_seized * 0.85;
    let c18_usd = c18_seized * 0.85;
    let ratio = if c6_usd > c18_usd {
        c6_usd / c18_usd
    } else {
        c18_usd / c6_usd
    };
    assert!(
        ratio < 1.5,
        "6-dec and 18-dec seizure should be ~equal. C6=${:.2}, C18=${:.2}, ratio={:.2}",
        c6_usd,
        c18_usd,
        ratio
    );

    std::println!(
        "\n  2x2 liquidation (4 unique decimals: 6,8,9,18):\n    C6 seized: {:.2} (${:.2})\n    C18 seized: {:.2} (${:.2})\n    D8 repaid: {:.6}\n    D9 repaid: {:.4}",
        c6_seized, c6_usd, c18_seized, c18_usd,
        d8_before - d8_after, d9_before - d9_after,
    );
}

#[test]
fn test_liquidation_4x4_eight_unique_decimals() {
    let mut t = LendingTest::new()
        .with_market(make_market("C6", 6, usd(1), 1_000_000.0))
        .with_market(make_market("C9", 9, usd(150), 100_000.0))
        .with_market(make_market("C12", 12, usd(10), 500_000.0))
        .with_market(make_market("C18", 18, usd(1), 1_000_000.0))
        .with_market(make_market("D7", 7, usd(1), 1_000_000.0))
        .with_market(make_market("D8", 8, usd(60_000), 100_000.0))
        .with_market(make_market("D10", 10, usd(5), 500_000.0))
        .with_market(make_market("D15", 15, usd(1), 1_000_000.0))
        .with_position_limits(4, 4)
        .build();

    t.supply(ALICE, "C6", 5_000.0);
    let acct = t.resolve_account_id(ALICE);
    t.supply_to(ALICE, acct, "C9", 33.3);
    t.supply_to(ALICE, acct, "C12", 500.0);
    t.supply_to(ALICE, acct, "C18", 5_000.0);

    t.borrow(ALICE, "D7", 3_500.0);
    t.borrow(ALICE, "D8", 0.058);
    t.borrow(ALICE, "D10", 700.0);
    t.borrow(ALICE, "D15", 3_500.0);

    t.assert_healthy(ALICE);

    t.set_price("C6", usd(1) * 85 / 100);
    t.set_price("C9", usd(150) * 85 / 100);
    t.set_price("C12", usd(10) * 85 / 100);
    t.set_price("C18", usd(1) * 85 / 100);
    t.advance_and_sync(1000);

    assert!(t.health_factor(ALICE) < 1.0, "Should be liquidatable");

    let c6_b = t.supply_balance(ALICE, "C6");
    let c9_b = t.supply_balance(ALICE, "C9");
    let c12_b = t.supply_balance(ALICE, "C12");
    let c18_b = t.supply_balance(ALICE, "C18");

    t.liquidate_multi(
        LIQUIDATOR,
        ALICE,
        &[("D7", 500.0), ("D8", 0.008), ("D10", 100.0), ("D15", 500.0)],
    );

    let c6_s = c6_b - t.supply_balance(ALICE, "C6");
    let c9_s = c9_b - t.supply_balance(ALICE, "C9");
    let c12_s = c12_b - t.supply_balance(ALICE, "C12");
    let c18_s = c18_b - t.supply_balance(ALICE, "C18");

    assert!(c6_s > 0.0, "C6 (6-dec) seized={}", c6_s);
    assert!(c9_s > 0.0, "C9 (9-dec) seized={}", c9_s);
    assert!(c12_s > 0.0, "C12 (12-dec) seized={}", c12_s);
    assert!(c18_s > 0.0, "C18 (18-dec) seized={}", c18_s);

    let c6_usd = c6_s * 0.85;
    let c9_usd = c9_s * 127.5;
    let c12_usd = c12_s * 8.5;
    let c18_usd = c18_s * 0.85;
    let total = c6_usd + c9_usd + c12_usd + c18_usd;

    for (name, val) in [
        ("C6", c6_usd),
        ("C9", c9_usd),
        ("C12", c12_usd),
        ("C18", c18_usd),
    ] {
        let pct = val / total * 100.0;
        assert!(
            pct > 15.0 && pct < 35.0,
            "{} should be ~25% of seizure, got {:.1}% (${:.2}/${:.2})",
            name,
            pct,
            val,
            total
        );
    }

    std::println!(
        "\n  4x4 liquidation (8 unique decimals: 6,7,8,9,10,12,15,18):\n    Seized: C6=${:.2} C9=${:.2} C12=${:.2} C18=${:.2} (total=${:.2})",
        c6_usd, c9_usd, c12_usd, c18_usd, total,
    );
}
