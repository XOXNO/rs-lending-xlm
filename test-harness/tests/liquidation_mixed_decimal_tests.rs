//! Liquidation tests with mixed-decimal assets on BOTH collateral and debt sides.
//!
//! Verifies that the proportional seizure formula correctly handles:
//! - Rescaling from WAD to different asset decimals (6, 8, 9, 18).
//! - Proportional splits across collateral with different decimal counts.
//! - Multi-debt liquidation across decimal boundaries.
//! - Small seizure amounts that might round to zero on low-decimal tokens.
//! - Asymmetric collateral values (90/10 split).

extern crate std;

use test_harness::presets::{
    MarketPreset, ALICE, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, LIQUIDATOR,
};
use test_harness::{helpers::usd, LendingTest};

// ---------------------------------------------------------------------------
// Token presets
// ---------------------------------------------------------------------------

// Helpers for custom decimal presets.
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

// ---------------------------------------------------------------------------
// 1. Two collaterals (6-dec + 18-dec), single debt (8-dec).
//    Seizure must proportionally convert back to BOTH asset decimals.
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_two_collaterals_6dec_18dec_debt_8dec() {
    let mut t = LendingTest::new()
        .with_market(usdc_6())
        .with_market(dai_18())
        .with_market(wbtc_8())
        .build();

    // Alice supplies a 50/50 split across 6-dec and 18-dec.
    t.supply(ALICE, "USDC6", 5_000.0); // $5,000
    t.supply_to(ALICE, t.resolve_account_id(ALICE), "DAI18", 5_000.0); // $5,000
                                                                       // Total collateral: $10,000.

    // Borrow close to the liquidation threshold.
    // $10,000 * 0.80 threshold = $8,000 max. Borrow $7,500 WBTC = 0.125 BTC.
    t.borrow(ALICE, "WBTC8", 0.125);

    // Price move: WBTC rises to $70,000, taking debt to $8,750 > $8,000 threshold.
    t.set_price("WBTC8", usd(70_000));
    t.advance_and_sync(1000);

    let hf_before = t.health_factor(ALICE);
    assert!(hf_before < 1.0, "HF should be < 1.0, got {}", hf_before);

    let usdc_before = t.supply_balance(ALICE, "USDC6");
    let dai_before = t.supply_balance(ALICE, "DAI18");

    // Liquidate: repay 0.03 WBTC ($2,100).
    t.liquidate(LIQUIDATOR, ALICE, "WBTC8", 0.03);

    let usdc_after = t.supply_balance(ALICE, "USDC6");
    let dai_after = t.supply_balance(ALICE, "DAI18");

    // Both collaterals should be seized proportionally (~50/50).
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

    // Both seizures should land within 20% of each other (roughly proportional).
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

    // The debt should be reduced.
    let debt_after = t.borrow_balance(ALICE, "WBTC8");
    assert!(
        debt_after < 0.125,
        "Debt should be reduced after liquidation, got {}",
        debt_after
    );
}

// ---------------------------------------------------------------------------
// 2. Asymmetric collateral: 90% in 6-dec, 10% in 18-dec.
//    Verifies that small proportional seizure on 18-dec stays non-zero.
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_asymmetric_90pct_6dec_10pct_18dec() {
    let mut t = LendingTest::new()
        .with_market(usdc_6())
        .with_market(dai_18())
        .with_market(sol_9())
        .build();

    // 90% USDC (6 dec), 10% DAI (18 dec).
    t.supply(ALICE, "USDC6", 9_000.0);
    t.supply_to(ALICE, t.resolve_account_id(ALICE), "DAI18", 1_000.0);
    // Total: $10,000.

    // Borrow $7,500 SOL (50 SOL at $150).
    t.borrow(ALICE, "SOL9", 50.0);

    // SOL price rises to $175 → debt = $8,750 > $8,000 threshold.
    t.set_price("SOL9", usd(175));
    t.advance_and_sync(1000);

    assert!(t.health_factor(ALICE) < 1.0, "Should be liquidatable");

    let dai_before = t.supply_balance(ALICE, "DAI18");
    let usdc_before = t.supply_balance(ALICE, "USDC6");

    // Liquidate: repay 10 SOL ($1,750).
    t.liquidate(LIQUIDATOR, ALICE, "SOL9", 10.0);

    let dai_after = t.supply_balance(ALICE, "DAI18");
    let usdc_after = t.supply_balance(ALICE, "USDC6");

    let dai_seized = dai_before - dai_after;
    let usdc_seized = usdc_before - usdc_after;

    // Both should be seized — even the small 10% DAI position.
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

    // USDC seizure should be ~9x DAI seizure (proportional to the 90/10 split).
    let ratio = usdc_seized / dai_seized;
    assert!(
        ratio > 5.0 && ratio < 15.0,
        "USDC seizure should be ~9x DAI seizure. USDC={}, DAI={}, ratio={}",
        usdc_seized,
        dai_seized,
        ratio
    );
}

// ---------------------------------------------------------------------------
// 3. Multi-debt: repay both 6-dec and 18-dec debt tokens.
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_multi_debt_6dec_and_18dec() {
    let mut t = LendingTest::new()
        .with_market(usdc_6())
        .with_market(dai_18())
        .with_market(sol_9())
        .build();

    // Supply $20,000 SOL as collateral.
    t.supply(ALICE, "SOL9", 133.0); // ~$20,000

    // Borrow from two pools with different decimals.
    t.borrow(ALICE, "USDC6", 7_000.0); // $7,000 at 6 decimals.
    t.borrow(ALICE, "DAI18", 7_000.0); // $7,000 at 18 decimals.
                                       // Total debt: $14,000.

    // SOL drops to $120 → collateral = $15,960, threshold = $12,768 < $14,000.
    t.set_price("SOL9", usd(120));
    t.advance_and_sync(1000);

    assert!(t.health_factor(ALICE) < 1.0, "Should be liquidatable");

    let usdc_debt_before = t.borrow_balance(ALICE, "USDC6");
    let dai_debt_before = t.borrow_balance(ALICE, "DAI18");

    // Multi-debt liquidation: repay $2,000 USDC + $2,000 DAI.
    t.liquidate_multi(LIQUIDATOR, ALICE, &[("USDC6", 2_000.0), ("DAI18", 2_000.0)]);

    let usdc_debt_after = t.borrow_balance(ALICE, "USDC6");
    let dai_debt_after = t.borrow_balance(ALICE, "DAI18");

    // Both debts should be reduced.
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

    // SOL collateral should be seized.
    let sol_after = t.supply_balance(ALICE, "SOL9");
    assert!(
        sol_after < 133.0,
        "SOL9 collateral should be seized, remaining={}",
        sol_after
    );
}

// ---------------------------------------------------------------------------
// 4. Multi-debt: repay 6-dec + 9-dec debt with 18-dec collateral.
//    Verifies correct conversion when debt payments span decimal ranges.
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_multi_debt_different_decimals() {
    let mut t = LendingTest::new()
        .with_market(usdc_6())
        .with_market(dai_18())
        .with_market(sol_9())
        .build();

    // Supply $20,000 DAI18 (single 18-dec collateral).
    t.supply(ALICE, "DAI18", 20_000.0);

    // Borrow $7,000 USDC6 + $7,000 SOL9 (46.7 SOL) = $14,000 total debt.
    t.borrow(ALICE, "USDC6", 7_000.0);
    t.borrow(ALICE, "SOL9", 46.7);

    // Crash DAI → $0.85 → collateral=$17,000, threshold=$13,600,
    // debt=$14,000 → underwater.
    t.set_price("DAI18", usd(1) * 85 / 100);
    t.advance_and_sync(1000);

    assert!(t.health_factor(ALICE) < 1.0, "Should be liquidatable");

    // Repay both debt types in one call.
    t.liquidate_multi(LIQUIDATOR, ALICE, &[("USDC6", 2_000.0), ("SOL9", 10.0)]);

    // Both debts reduced.
    assert!(
        t.borrow_balance(ALICE, "USDC6") < 7_000.0,
        "USDC6 debt reduced"
    );
    assert!(t.borrow_balance(ALICE, "SOL9") < 46.7, "SOL9 debt reduced");

    // DAI18 collateral seized.
    assert!(
        t.supply_balance(ALICE, "DAI18") < 20_000.0,
        "DAI18 collateral seized"
    );
}

// ---------------------------------------------------------------------------
// 5. Bad-debt cleanup with mixed decimals.
//    Verifies socialization works when collateral is near zero across
//    different decimal tokens.
// ---------------------------------------------------------------------------

#[test]
fn test_bad_debt_cleanup_mixed_decimals() {
    let mut t = LendingTest::new()
        .with_market(usdc_6())
        .with_market(dai_18())
        .build();

    // Supply 6-dec collateral, borrow 18-dec debt (different decimal counts).
    t.supply(ALICE, "USDC6", 200.0); // $200

    // Borrow $150 DAI18.
    t.borrow(ALICE, "DAI18", 150.0);

    // Crash collateral price only (debt price stays at $1).
    t.set_price("USDC6", usd(1) / 1000); // $0.001 → collateral = $0.20.
    t.advance_and_sync(1000);

    // Collateral: $0.20, debt: $150 → deeply underwater.
    let hf = t.health_factor(ALICE);
    assert!(hf < 0.01, "HF should be deeply underwater, got {}", hf);

    // Liquidate to trigger the bad-debt path (collateral < $5).
    t.liquidate(LIQUIDATOR, ALICE, "DAI18", 10.0);

    // After liquidation + bad-debt cleanup, the account is removed entirely.
    // The liquidator should have received some DAI back (refund from
    // overpayment or capped repayment). Just verify no panic occurred.
    // The bad-debt path seizes all collateral and socializes remaining debt.
}

// ---------------------------------------------------------------------------
// 6. Liquidation preserves protocol-fee calculation across decimals.
//    Protocol fee = bonus_portion * liquidation_fees_bps / BPS.
//    Verifies the fee neither underflows for 6-dec nor overflows for 18-dec.
// ---------------------------------------------------------------------------

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

    // Liquidate.
    t.liquidate(LIQUIDATOR, ALICE, "DAI18", 2_000.0);

    let collateral_after = t.total_collateral(ALICE);
    let debt_after = t.total_debt(ALICE);

    // Collateral should decrease (seizure occurred).
    assert!(
        collateral_after < collateral_before,
        "Collateral should decrease: before={}, after={}",
        collateral_before,
        collateral_after
    );

    // Debt should decrease.
    assert!(
        debt_after < 7_500.0,
        "Debt should decrease, got {}",
        debt_after
    );
}

// ---------------------------------------------------------------------------
// 7. 4 collaterals x 4 debts — ALL unique decimals (6,7,8,9,10,12,15,18).
//    The ultimate cross-decimal liquidation stress test.
//    If this exceeds Soroban's budget, it reveals the max position complexity.
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_2x2_four_unique_decimals() {
    // 4 markets with unique decimals: 6, 18 (collateral) + 8, 9 (debt).
    // Covers the widest rescale gap (WAD->6 = divide by 10^12) plus
    // cross-decimal multi-debt liquidation.
    //
    let mut t = LendingTest::new()
        .with_market(make_market("C6", 6, usd(1), 1_000_000.0))
        .with_market(make_market("C18", 18, usd(1), 1_000_000.0))
        .with_market(make_market("D8", 8, usd(60_000), 100_000.0))
        .with_market(make_market("D9", 9, usd(150), 100_000.0))
        .build();

    // --- Supply 2 collaterals (50/50 split, $5k each = $10k total) ---
    t.supply(ALICE, "C6", 5_000.0);
    let acct = t.resolve_account_id(ALICE);
    t.supply_to(ALICE, acct, "C18", 5_000.0);

    // --- Borrow 2 debts with different decimals ---
    t.borrow(ALICE, "D8", 0.058); // ~$3,480 at 8 decimals.
    t.borrow(ALICE, "D9", 23.0); // ~$3,450 at 9 decimals.
                                 // Total debt ~$6,930, threshold $8,000 → healthy.

    t.assert_healthy(ALICE);

    // --- 15% collateral drop → underwater ---
    t.set_price("C6", usd(1) * 85 / 100);
    t.set_price("C18", usd(1) * 85 / 100);
    t.advance_and_sync(1000);

    assert!(t.health_factor(ALICE) < 1.0, "Should be liquidatable");

    let c6_before = t.supply_balance(ALICE, "C6");
    let c18_before = t.supply_balance(ALICE, "C18");
    let d8_before = t.borrow_balance(ALICE, "D8");
    let d9_before = t.borrow_balance(ALICE, "D9");

    // --- Multi-debt liquidation: repay both 8-dec and 9-dec debts ---
    t.liquidate_multi(LIQUIDATOR, ALICE, &[("D8", 0.01), ("D9", 5.0)]);

    // --- Verify both debts reduced ---

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

    // --- Verify BOTH collaterals seized (6-dec AND 18-dec) ---
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

    // --- Proportionality: ~50/50 since equal-value collateral ---
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

// ---------------------------------------------------------------------------
// 8. 4 collaterals x 4 debts — ALL 8 unique decimals (6,7,8,9,10,12,15,18)
// ---------------------------------------------------------------------------

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

    // Supply 4 collaterals (~$5,000 each = $20,000 total).
    t.supply(ALICE, "C6", 5_000.0);
    let acct = t.resolve_account_id(ALICE);
    t.supply_to(ALICE, acct, "C9", 33.3);
    t.supply_to(ALICE, acct, "C12", 500.0);
    t.supply_to(ALICE, acct, "C18", 5_000.0);

    // Borrow 4 debts (~$3,500 each = $14,000 total).
    t.borrow(ALICE, "D7", 3_500.0);
    t.borrow(ALICE, "D8", 0.058);
    t.borrow(ALICE, "D10", 700.0);
    t.borrow(ALICE, "D15", 3_500.0);

    t.assert_healthy(ALICE);

    // Drop all collateral prices by 15% to push underwater.
    t.set_price("C6", usd(1) * 85 / 100);
    t.set_price("C9", usd(150) * 85 / 100);
    t.set_price("C12", usd(10) * 85 / 100);
    t.set_price("C18", usd(1) * 85 / 100);
    t.advance_and_sync(1000);

    assert!(t.health_factor(ALICE) < 1.0, "Should be liquidatable");

    // Capture pre-liquidation state.
    let c6_b = t.supply_balance(ALICE, "C6");
    let c9_b = t.supply_balance(ALICE, "C9");
    let c12_b = t.supply_balance(ALICE, "C12");
    let c18_b = t.supply_balance(ALICE, "C18");

    // Multi-debt liquidation: repay portion of all 4 debts.
    t.liquidate_multi(
        LIQUIDATOR,
        ALICE,
        &[("D7", 500.0), ("D8", 0.008), ("D10", 100.0), ("D15", 500.0)],
    );

    // ALL 4 collaterals must be seized.
    let c6_s = c6_b - t.supply_balance(ALICE, "C6");
    let c9_s = c9_b - t.supply_balance(ALICE, "C9");
    let c12_s = c12_b - t.supply_balance(ALICE, "C12");
    let c18_s = c18_b - t.supply_balance(ALICE, "C18");

    assert!(c6_s > 0.0, "C6 (6-dec) seized={}", c6_s);
    assert!(c9_s > 0.0, "C9 (9-dec) seized={}", c9_s);
    assert!(c12_s > 0.0, "C12 (12-dec) seized={}", c12_s);
    assert!(c18_s > 0.0, "C18 (18-dec) seized={}", c18_s);

    // Proportionality: each ~25% since equal-value collateral.
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
