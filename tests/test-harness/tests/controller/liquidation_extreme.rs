//! Extreme and edge-case liquidation scenarios: custom spoke curves, extreme
//! prices and bonuses, high-LTV/low-bonus and low-threshold/high-bonus spokes,
//! low and mixed decimals, the full health-factor distance spectrum, the dust
//! guard, and the toxic (solvent-but-unhealthy) band.

use controller::constants::WAD;
use governance::op::{AdminOperation, SpokeLiquidationCurveArgs};
use test_harness::helpers::{usd, usd_cents};
use test_harness::presets::{
    AssetConfigPreset, MarketPreset, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS,
};
use test_harness::{LendingTest, ALICE, HARNESS_SPOKE, LIQUIDATOR};

/// Overrides the harness spoke's liquidation curve (target HF, HF for max bonus,
/// bonus factor) through the governance forwarder.
fn set_curve(t: &LendingTest, target_hf_wad: i128, hf_for_max_bonus_wad: i128, factor_bps: u32) {
    let admin = t.admin();
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
            spoke_id: HARNESS_SPOKE,
            target_hf_wad,
            hf_for_max_bonus_wad,
            liquidation_bonus_factor_bps: factor_bps,
        }),
    );
}

/// A market preset with caller-chosen decimals, price, and risk params.
fn asset(
    name: &'static str,
    decimals: u32,
    price_wad: i128,
    ltv: u32,
    threshold: u32,
    bonus: u32,
    liquidity: f64,
) -> MarketPreset {
    MarketPreset {
        name,
        decimals,
        price_wad,
        initial_liquidity: liquidity,
        config: AssetConfigPreset {
            loan_to_value: ltv,
            liquidation_threshold: threshold,
            liquidation_bonus: bonus,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// A $1 stable debt market (7 decimals, high LTV so it is freely borrowable).
fn stable(name: &'static str) -> MarketPreset {
    asset(name, 7, usd(1), 9000, 9500, 200, 100_000_000.0)
}

/// Liquidates `amount` of `debt_asset` and returns
/// `(collateral_usd_received, debt_usd_repaid, profit_ratio)` where the ratio is
/// `1 + realized_bonus`. `coll_asset` price is fixed during the call.
fn liquidate_measure(
    t: &mut LendingTest,
    debt_asset: &str,
    amount: f64,
    coll_asset: &str,
    coll_price_usd: f64,
) -> (f64, f64, f64) {
    t.get_or_create_user(LIQUIDATOR);
    let coll_before = t.token_balance(LIQUIDATOR, coll_asset);
    let debt_before = t.total_debt(ALICE);
    t.liquidate(LIQUIDATOR, ALICE, debt_asset, amount);
    let coll_usd = (t.token_balance(LIQUIDATOR, coll_asset) - coll_before) * coll_price_usd;
    let debt_usd = debt_before - t.total_debt(ALICE);
    assert!(
        debt_usd > 0.0 && coll_usd > 0.0,
        "liquidation moved no value: coll_usd={coll_usd} debt_usd={debt_usd}"
    );
    (coll_usd, debt_usd, coll_usd / debt_usd)
}

// ---------------------------------------------------------------------------
// Custom spoke curves
// ---------------------------------------------------------------------------

// A high target HF (5.0) drives the liquidation to repay far more debt to lift
// the account well above 1.0; the position must end much healthier or closed.
#[test]
fn test_extreme_high_target_hf_curve() {
    let mut t = LendingTest::new()
        .with_market(asset("VOL", 7, usd(100), 7000, 8000, 500, 1_000_000.0))
        .with_market(stable("USD"))
        .build();
    set_curve(&t, 5 * WAD, 2 * WAD, 10_000);

    t.supply(ALICE, "VOL", 100.0); // $10,000
    t.borrow(ALICE, "USD", 6_000.0);
    t.set_price("VOL", usd(70)); // collateral -> $7,000, weighted $5,600 < $6,000 -> HF < 1
    t.advance_and_sync(100);
    t.assert_liquidatable(ALICE);
    let hf_before = t.health_factor(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "USD", 6_000.0);
    let healed = t.find_account_id(ALICE).is_none() || t.health_factor(ALICE) > hf_before + 0.5;
    assert!(healed, "high target HF must produce a large heal");
}

// A near-zero bonus factor keeps the realized bonus at the base even when the
// account is deeply underwater: the curve is effectively flat.
#[test]
fn test_flat_bonus_factor_stays_at_base() {
    let mut t = LendingTest::new()
        .with_market(asset("VOL", 7, usd(100), 7000, 8000, 500, 1_000_000.0))
        .with_market(stable("USD"))
        .build();
    set_curve(&t, 1_020_000_000_000_000_000, 510_000_000_000_000_000, 1); // factor 0.01%

    t.supply(ALICE, "VOL", 100.0); // $10,000
    t.borrow(ALICE, "USD", 5_000.0);
    t.set_price("VOL", usd(40)); // deep: collateral $4,000, weighted $3,200 << $5,000
    t.advance_and_sync(100);
    t.assert_liquidatable(ALICE);

    let (_c, _d, ratio) = liquidate_measure(&mut t, "USD", 1_000.0, "VOL", 40.0);
    // Base bonus is 5%; a flat factor must keep the realized bonus near it.
    assert!(
        ratio < 1.08,
        "flat curve must stay near the 5% base bonus, got {ratio}"
    );
}

// A narrow-band curve (bonus reaches max just below target) still respects the
// seizure-safety ceiling: the realized bonus never exceeds the per-threshold max
// (25% at threshold 0.80), so the liquidator can never over-seize.
#[test]
fn test_narrow_curve_bonus_bounded_by_max() {
    let mut t = LendingTest::new()
        .with_market(asset("VOL", 7, usd(100), 7000, 8000, 500, 1_000_000.0))
        .with_market(stable("USD"))
        .build();
    // target 1.05, max-bonus at 1.049: a 0.001 band.
    set_curve(&t, 1_050_000_000_000_000_000, 1_049_000_000_000_000_000, 10_000);

    t.supply(ALICE, "VOL", 100.0); // $10,000
    t.borrow(ALICE, "USD", 6_900.0); // at the 0.70 LTV cap
    t.set_price("VOL", usd(55)); // underwater
    t.advance_and_sync(100);
    t.assert_liquidatable(ALICE);

    let (_c, _d, ratio) = liquidate_measure(&mut t, "USD", 500.0, "VOL", 55.0);
    assert!(
        ratio > 1.0 && ratio <= 1.26,
        "realized bonus must stay within [0, max=25%], got {ratio}"
    );
}

// ---------------------------------------------------------------------------
// Extreme thresholds / bonuses
// ---------------------------------------------------------------------------

// High LTV, high threshold, tiny 1% bonus (stablecoin-style): a shallow depeg
// liquidation must pay only ~1% and never over-seize.
#[test]
fn test_high_ltv_low_bonus_stablecoin() {
    let mut t = LendingTest::new()
        .with_market(asset("STA", 7, usd(1), 9500, 9700, 100, 100_000_000.0))
        .with_market(stable("USD"))
        .build();

    t.supply(ALICE, "STA", 10_000.0); // $10,000
    t.borrow(ALICE, "USD", 9_400.0);
    t.set_price("STA", usd_cents(96)); // depeg -> HF < 1
    t.advance_and_sync(100);
    t.assert_liquidatable(ALICE);

    let (_c, _d, ratio) = liquidate_measure(&mut t, "USD", 2_000.0, "STA", 0.96);
    assert!(
        ratio > 1.005 && ratio < 1.02,
        "stablecoin liquidation must pay ~1% bonus, got {ratio}"
    );
}

// Zero base bonus with a flat curve: the liquidator receives collateral equal in
// value to the debt repaid (no profit, no over-seizure).
#[test]
fn test_zero_bonus_liquidation() {
    let mut t = LendingTest::new()
        .with_market(asset("VOL", 7, usd(100), 7000, 8000, 0, 1_000_000.0))
        .with_market(stable("USD"))
        .build();
    set_curve(&t, 1_020_000_000_000_000_000, 510_000_000_000_000_000, 1);

    t.supply(ALICE, "VOL", 100.0); // $10,000
    t.borrow(ALICE, "USD", 7_000.0);
    t.set_price("VOL", usd(85)); // HF < 1
    t.advance_and_sync(100);
    t.assert_liquidatable(ALICE);

    let (_c, _d, ratio) = liquidate_measure(&mut t, "USD", 1_000.0, "VOL", 85.0);
    assert!(
        (0.98..=1.02).contains(&ratio),
        "zero bonus must seize ~1:1 in value, got {ratio}"
    );
}

// ---------------------------------------------------------------------------
// Toxic band (solvent but HF < 1) — the F1 fix, end to end
// ---------------------------------------------------------------------------

// A low-threshold (0.45) collateral position that is still solvent (neutral
// collateral > debt) but HF < 1 must be fully healed by a base-bonus liquidation
// with NO socialized bad debt: the borrower keeps collateral and owes nothing.
#[test]
fn test_toxic_band_low_threshold_no_bad_debt() {
    let mut t = LendingTest::new()
        .with_market(asset("VOL", 7, usd(100), 4000, 4500, 500, 1_000_000.0))
        .with_market(stable("USD"))
        .build();

    t.supply(ALICE, "VOL", 100.0); // $10,000
    t.borrow(ALICE, "USD", 3_900.0); // within 0.40 LTV
    t.set_price("VOL", usd(60)); // collateral -> $6,000 > $3,900 debt, HF ~0.69
    t.advance_and_sync(100);
    assert!(t.total_collateral(ALICE) > t.total_debt(ALICE), "solvent");
    t.assert_liquidatable(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "USD", 3_900.0);

    assert!(t.total_debt(ALICE) < 1.0, "debt fully cleared, no bad debt");
    assert!(
        t.total_collateral(ALICE) > 100.0,
        "borrower keeps collateral instead of a full seizure"
    );
    // Nothing left to socialize.
    let id = t.resolve_account_id(ALICE);
    assert!(t.try_clean_bad_debt_by_id(id).is_err());
}

// Toxic band with a mix of collateral decimals (3-dec + 18-dec). The healing
// liquidation must seize proportionally across both and still leave no bad debt.
#[test]
fn test_toxic_band_multi_collateral_mixed_decimals() {
    let mut t = LendingTest::new()
        .with_market(asset("LOW3", 3, usd(1_000), 4000, 4500, 500, 1_000_000.0))
        .with_market(asset("HI18", 18, usd(1), 4000, 4500, 500, 10_000_000.0))
        .with_market(stable("USD"))
        .build();

    t.supply(ALICE, "LOW3", 5.0); // $5,000 at 3 decimals
    let id = t.resolve_account_id(ALICE);
    t.supply_to(ALICE, id, "HI18", 5_000.0); // $5,000 at 18 decimals
    t.borrow(ALICE, "USD", 3_900.0); // within LTV on $10,000

    t.set_prices(&[("LOW3", usd(600)), ("HI18", usd_cents(60))]); // both -60% -> $6,000
    t.advance_and_sync(100);
    assert!(t.total_collateral(ALICE) > t.total_debt(ALICE), "solvent");
    t.assert_liquidatable(ALICE);

    let low3_before = t.supply_balance(ALICE, "LOW3");
    let hi18_before = t.supply_balance(ALICE, "HI18");
    t.liquidate(LIQUIDATOR, ALICE, "USD", 3_900.0);

    assert!(t.total_debt(ALICE) < 1.0, "debt fully cleared, no bad debt");
    assert!(
        t.supply_balance(ALICE, "LOW3") < low3_before
            && t.supply_balance(ALICE, "HI18") < hi18_before,
        "both collaterals seized proportionally"
    );
}

// ---------------------------------------------------------------------------
// Dust guard, end to end
// ---------------------------------------------------------------------------

// A liquidation sized so the target-HF partial would leave a sub-$5 debt
// remainder must escalate to a full close: the account ends with zero debt.
#[test]
fn test_dust_debt_escalates_to_full_close() {
    let mut t = LendingTest::new()
        .with_market(asset("VOL", 7, usd(100), 7000, 8000, 500, 1_000_000.0))
        .with_market(stable("USD"))
        .build();

    t.supply(ALICE, "VOL", 100.0); // $10,000
    t.borrow(ALICE, "USD", 6.0); // tiny debt: any partial leaves < $5
    t.set_price("VOL", usd_cents(7)); // crush collateral so HF < 1
    t.advance_and_sync(100);
    t.assert_liquidatable(ALICE);

    // Offer to repay the whole debt; the dust guard makes the full close the plan.
    t.liquidate(LIQUIDATOR, ALICE, "USD", 6.0);
    assert!(
        t.total_debt(ALICE) < 0.01,
        "sub-floor remainder escalated to a full debt close, got {}",
        t.total_debt(ALICE)
    );
}

// ---------------------------------------------------------------------------
// Deep underwater -> socialized bad debt
// ---------------------------------------------------------------------------

// A collateral crash that leaves debt far above a near-zero collateral produces
// socializable bad debt: the cleanup succeeds and removes the account.
#[test]
fn test_deep_crash_socializes_bad_debt() {
    let mut t = LendingTest::new()
        .with_market(asset("VOL", 7, usd(100), 7000, 8000, 500, 1_000_000.0))
        .with_market(stable("USD"))
        .build();

    t.supply(ALICE, "VOL", 100.0); // $10,000
    t.borrow(ALICE, "USD", 7_000.0);
    t.set_price("VOL", usd_cents(3)); // collateral -> ~$3, debt $7,000
    t.advance_and_sync(100);
    t.assert_liquidatable(ALICE);

    let id = t.resolve_account_id(ALICE);
    t.clean_bad_debt_by_id(id);
    assert!(t.find_account_id(ALICE).is_none(), "account cleaned away");
}

// ---------------------------------------------------------------------------
// Decimals and prices
// ---------------------------------------------------------------------------

// A high-value, low-decimal (3) asset (1 milli-unit ~ $60) liquidates cleanly and
// the liquidator's bonus stays within the expected band.
#[test]
fn test_high_value_low_decimal_collateral() {
    let mut t = LendingTest::new()
        .with_market(asset("K3", 3, usd(60_000), 7000, 8000, 500, 100_000.0))
        .with_market(stable("USD"))
        .build();

    t.supply(ALICE, "K3", 1.0); // $60,000
    t.borrow(ALICE, "USD", 41_000.0);
    t.set_price("K3", usd(50_000)); // HF < 1
    t.advance_and_sync(100);
    t.assert_liquidatable(ALICE);

    let (_c, _d, ratio) = liquidate_measure(&mut t, "USD", 5_000.0, "K3", 50_000.0);
    assert!(
        ratio > 1.0 && ratio < 1.30,
        "low-decimal high-value liquidation stays within bonus bounds, got {ratio}"
    );
    assert!(t.health_factor(ALICE) > 0.0);
}

// Extreme decimal spread in one account: 3-decimal collateral, 18-decimal debt.
#[test]
fn test_extreme_decimal_spread_3_collateral_18_debt() {
    let mut t = LendingTest::new()
        .with_market(asset("C3", 3, usd(1_000), 7000, 8000, 500, 1_000_000.0))
        .with_market(asset("D18", 18, usd(1), 9000, 9500, 200, 100_000_000.0))
        .build();

    t.supply(ALICE, "C3", 10.0); // $10,000
    t.borrow(ALICE, "D18", 7_000.0);
    t.set_price("C3", usd(850)); // HF < 1
    t.advance_and_sync(100);
    t.assert_liquidatable(ALICE);
    let hf_before = t.health_factor(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "D18", 2_000.0);
    assert!(
        t.find_account_id(ALICE).is_none() || t.health_factor(ALICE) > hf_before,
        "cross-decimal liquidation must improve health"
    );
}

// ---------------------------------------------------------------------------
// Health-factor distance spectrum
// ---------------------------------------------------------------------------

// Across the HF spectrum (shallow to deep), every liquidation succeeds and moves
// positive value within the seizure-safety ceiling. A single partial bite may
// raise or lower HF (bounded across chains by the anti-ratchet invariant), so
// only the bonus ceiling is asserted here.
#[test]
fn test_hf_spectrum_liquidations_bounded() {
    let prices = [usd(79), usd(70), usd(55), usd(45)];
    for &price in &prices {
        let mut t = LendingTest::new()
            .with_market(asset("VOL", 7, usd(100), 7000, 8000, 500, 1_000_000.0))
            .with_market(stable("USD"))
            .build();
        t.supply(ALICE, "VOL", 100.0); // $10,000
        t.borrow(ALICE, "USD", 6_900.0);
        t.set_price("VOL", price);
        t.advance_and_sync(100);
        t.assert_liquidatable(ALICE);

        let coll_price = price as f64 / WAD as f64;
        let (_c, _d, ratio) = liquidate_measure(&mut t, "USD", 500.0, "VOL", coll_price);
        assert!(
            ratio > 1.0 && ratio <= 1.26,
            "bonus within [0, max=25%] at price {coll_price}, got {ratio}"
        );
    }
}
