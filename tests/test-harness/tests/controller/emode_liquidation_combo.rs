use test_harness::{
    eth_preset, usd_cents, usdc_preset, usdt_stable_preset, LendingTest, ALICE, BOB, LIQUIDATOR,
    STABLECOIN_EMODE,
};
// 1. E-mode threshold supersedes asset threshold for HF

// USDC asset threshold = 80 %. Stablecoin e-mode threshold = 98 %.
// Position that is *liquidatable* in standard mode is still *healthy*
// in e-mode for the same price drop because the e-mode threshold
// expands the safe band.
#[test]
fn test_emode_threshold_supersedes_asset_threshold() {
    let mut standard = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .build();

    standard.supply(ALICE, "USDC", 10_000.0);
    // Borrow $7500 USDT — HF (standard, threshold 80 %) = 8000/7500 = 1.067.
    standard.borrow(ALICE, "USDT", 7_500.0);
    // Drop USDC to $0.93: threshold-weighted = $7440 < $7500 → HF < 1 in
    // standard mode.
    standard.set_price("USDC", usd_cents(93));
    standard.assert_liquidatable(ALICE);

    // Same setup under e-mode — should be healthy because the e-mode
    // threshold is 98 %, not 80 %. Weighted = 10_000 * 0.93 * 0.98 =
    // $9114 > $7500.
    let mut emode = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();
    emode.create_emode_account(ALICE, 1);
    emode.supply(ALICE, "USDC", 10_000.0);
    emode.borrow(ALICE, "USDT", 7_500.0);
    emode.set_price("USDC", usd_cents(93));
    emode.assert_healthy(ALICE);
}
// 2. E-mode bonus consistency under deeper crashes

// Under a deep crash that triggers e-mode liquidation, the realized
// bonus must stay bounded above by the e-mode max — far below the
// protocol's standard 15 % cap. Pins that the engine uses the e-mode
// bonus parameter, not the per-asset bonus.
#[test]
fn test_emode_bonus_bounded_by_category_bonus() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .with_dust_disabled_all_markets()
        .build();

    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 9_500.0);
    // Force deep crash — way past the standard liquidation threshold,
    // but still inside e-mode partial-liquidation band.
    t.set_price("USDC", usd_cents(85));
    t.assert_liquidatable(ALICE);

    t.get_or_create_user(LIQUIDATOR);
    let usdc_before = t.token_balance(LIQUIDATOR, "USDC");
    t.liquidate(LIQUIDATOR, ALICE, "USDT", 500.0);
    let usdc_after = t.token_balance(LIQUIDATOR, "USDC");

    let usdc_received = usdc_after - usdc_before;
    let usd_received = usdc_received * 0.85;
    let realized_bonus = (usd_received / 500.0) - 1.0;

    // E-mode bonus (STABLECOIN_EMODE) = 200 bps = 2 %. Engine clamps
    // realized bonus at or below this; allow modest arithmetic slop.
    assert!(
        realized_bonus <= 0.03,
        "realized bonus in e-mode must stay near 2 % (e-mode cap), got {:.4}",
        realized_bonus
    );
    assert!(
        realized_bonus >= 0.005,
        "realized bonus should not be zero / negative, got {:.4}",
        realized_bonus
    );
}
// 3. Two-asset same-category collateral liquidation

// Position with collateral split across two e-mode assets (USDC + USDT)
// and debt in USDT. Liquidation seizes USDC (the collateral side) and
// reduces USDT debt. Pins the collateral-side iteration in the bulk
// liquidation path: the engine must pick a viable collateral asset
// from the supply side that is not also part of the debt repayment.
#[test]
fn test_emode_liquidation_with_split_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .with_dust_disabled_all_markets()
        .build();

    // Bob supplies the borrow-side liquidity so Alice's USDT borrow
    // doesn't trip the utilization cap. (The pool's `initial_liquidity`
    // is minted as tokens but not registered as supplied, so utilization
    // only counts user supplies.)
    t.create_emode_account(BOB, 1);
    t.supply(BOB, "USDT", 100_000.0);

    t.create_emode_account(ALICE, 1);
    // Two-sided collateral: $5000 USDC + $4000 USDT supply. Borrow
    // $8000 USDT.
    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "USDT", 4_000.0);
    t.borrow(ALICE, "USDT", 8_000.0);
    // Drop USDC to push HF underwater. With e-mode threshold 98 %:
    // weighted = (5000 * price * 0.98) + (4000 * 0.98) ≥ 8000 at
    // healthy prices; at USDC=$0.60 → 5000*0.60*0.98 + 4000*0.98 =
    // 2940 + 3920 = 6860 < 8000 → underwater.
    t.set_price("USDC", usd_cents(60));
    t.assert_liquidatable(ALICE);

    let usdc_collat_before = t.supply_balance(ALICE, "USDC");
    t.liquidate(LIQUIDATOR, ALICE, "USDT", 500.0);
    let usdc_collat_after = t.supply_balance(ALICE, "USDC");

    // USDC collateral must have decreased (liquidator seized it).
    assert!(
        usdc_collat_after < usdc_collat_before,
        "USDC collateral must decrease after liquidation: before={:.4}, after={:.4}",
        usdc_collat_before,
        usdc_collat_after
    );
}
// 4. Non-e-mode collateral cannot be added to e-mode account

// Pins that the e-mode supply gate rejects non-category assets even
// after the position has been opened with category-allowed collateral.
// The existing `test_emode_rejects_non_category_supply` test covers
// rejection from the start; this variant exercises the gate on a
// position that already has e-mode positions open.
#[test]
fn test_emode_rejects_non_category_collateral_addition() {
    use test_harness::{assert_contract_error, errors};

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        // ETH intentionally not in the category.
        .build();

    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 1_000.0);

    // Adding ETH (non-category) must be rejected even though the
    // account already has e-mode collateral.
    let result = t.try_supply(ALICE, "ETH", 0.1);
    assert_contract_error(result, errors::ASSET_NOT_SUPPORTED);
}
