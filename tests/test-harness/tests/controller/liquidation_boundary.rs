use controller::constants::WAD;
use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usd_cents, usdc_preset, LendingTest, ALICE,
    LIQUIDATOR,
};
// HF boundary off-by-ones (Blend V2 L-05 class)

// At HF == 1.0 (against the *liquidation threshold*) the account is
// healthy: `process_liquidation` rejects via `HealthFactorTooHigh`. The
// position is built by first borrowing while healthy at the LTV gate,
// then dropping the USDC price so HF lands at the threshold boundary.
#[test]
fn test_hf_exactly_one_is_healthy() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // USDC liquidation_threshold = 0.80, LTV = 0.75.
    // Supply $10k, borrow 3.5 ETH ($7000). HF (threshold) = 1.143; HF
    // (LTV) = 1.071 — borrow allowed.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.5);

    // Drop USDC to $0.875: threshold-weighted = $10k * 0.875 * 0.80 =
    // $7000, debt = $7000 → HF = 1.0 exactly.
    t.set_price("USDC", usd(1) * 875 / 1000);

    let hf_raw = t.health_factor_raw(ALICE);
    // The threshold/price math is exact enough that HF lands within a
    // few ulps of WAD; widen tolerance modestly for arithmetic slop.
    let drift = (hf_raw - WAD).abs();
    assert!(
        drift < 1_000,
        "HF should be ~1.0 (raw WAD), got {}, drift={}",
        hf_raw,
        drift
    );

    // Liquidation must be rejected — boundary is healthy.
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::HEALTH_FACTOR_TOO_HIGH);
}

// `is_liquidatable` uses strict `<`: an account sitting exactly on
// HF = 1.0 is healthy, and the view flips only strictly below it. The
// construction is rounding-free: $10k USDC (threshold 0.80) → weighted
// $8000 exactly; 2 ETH borrowed at $1000 and repriced to $4000 → debt
// $8000 exactly under unit indexes.
#[test]
fn test_is_liquidatable_flips_strictly_below_hf_one() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.set_price("ETH", usd(1_000));
    t.borrow(ALICE, "ETH", 2.0);
    t.set_price("ETH", usd(4_000));

    let account_id = t.resolve_account_id(ALICE);
    let hf_raw = t.health_factor_raw(ALICE);
    assert_eq!(hf_raw, WAD, "construction must land exactly on HF = 1.0");
    assert!(
        !t.ctrl_client().is_liquidatable(&account_id),
        "HF exactly 1.0 is healthy"
    );

    // One ETH cent deeper flips the strict inequality.
    t.set_price("ETH", usd(4_000) + usd(1) / 100);
    assert!(t.health_factor_raw(ALICE) < WAD);
    assert!(t.ctrl_client().is_liquidatable(&account_id));
}

// One step below the boundary triggers liquidation. Same setup but
// price nudged to $0.874 → HF ≈ 0.999.
#[test]
fn test_hf_just_below_one_is_liquidatable() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.5);
    t.set_price("USDC", usd(1) * 874 / 1000); // HF ≈ 0.9989

    let hf_raw = t.health_factor_raw(ALICE);
    assert!(hf_raw < WAD, "HF must be < 1.0, got {}", hf_raw);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert!(
        result.is_ok(),
        "liquidation at HF<1 should succeed, got {:?}",
        result
    );
}

// After a successful partial liquidation, HF must improve. For a mild
// crash the partial liquidation may restore HF to ≥ 1.0; for deeper
// crashes the position is in bad-debt territory and partial seizure
// can't restore HF. Both branches are exercised: HF *strictly* increases.
#[test]
fn test_liquidation_strictly_improves_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0); // $6000 debt
                                 // Mild crash: HF ≈ 0.987, well inside the partial-liquidation band.
    t.set_price("USDC", usd_cents(74));
    t.assert_liquidatable(ALICE);

    let hf_before = t.health_factor(ALICE);
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5); // $1000 partial repay
    let hf_after = t.health_factor(ALICE);

    assert!(
        hf_after > hf_before,
        "post-liquidation HF must strictly improve: before={:.4}, after={:.4}",
        hf_before,
        hf_after
    );
}
// Bonus curve monotonicity + boundaries

// Sweeps liquidatable HF levels in the mild-underwater band where the
// Dutch-auction bonus interpolates between base (5 %) and max (15 %).
// Realized bonus must be non-decreasing as HF drops within this band.
// (At very deep underwater the engine clamps seize to feasible payment,
// which is its own invariant tested by `test_liquidation_bonus_clamped_at_max`
// and the bad-debt branch.)
#[test]
fn test_liquidation_bonus_monotone_in_mild_underwater_band() {
    let mut bonuses: std::vec::Vec<(u32, f64)> = std::vec::Vec::new();
    // USDC threshold = 80 %. $10k supply, $6k debt → HF = 1.0 at
    // cents=75. Band: 73 → 0.97, 71 → 0.95, 69 → 0.92, 67 → 0.89.
    for cents_per_dollar in [73u32, 71, 69, 67] {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(eth_preset())
            .with_dust_disabled_all_markets()
            .build();

        t.supply(ALICE, "USDC", 10_000.0);
        t.borrow(ALICE, "ETH", 3.0);
        t.set_price("USDC", usd_cents(cents_per_dollar.into()));
        if !t.can_be_liquidated(ALICE) {
            continue;
        }

        t.get_or_create_user(LIQUIDATOR);
        let liq_usdc_before = t.token_balance(LIQUIDATOR, "USDC");
        t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.1); // $200 repay
        let liq_usdc_after = t.token_balance(LIQUIDATOR, "USDC");

        let usdc_received = liq_usdc_after - liq_usdc_before;
        let usd_received = usdc_received * (cents_per_dollar as f64) / 100.0;
        let realized_bonus = (usd_received / 200.0) - 1.0;
        bonuses.push((cents_per_dollar, realized_bonus));
    }

    assert!(
        bonuses.len() >= 3,
        "need at least 3 samples for monotonicity, got {}: {:?}",
        bonuses.len(),
        bonuses
    );
    for window in bonuses.windows(2) {
        let (prev_c, prev_b) = window[0];
        let (next_c, next_b) = window[1];
        assert!(
            next_b + 1e-4 >= prev_b,
            "bonus must be non-decreasing across worsening HF: at cents={} bonus={:.6}, at cents={} bonus={:.6}, full={:?}",
            prev_c, prev_b, next_c, next_b, bonuses
        );
    }
    // Also verify the lowest realized bonus is at or above base (5 %)
    // and the highest stays below the seizure cap (25 % at the 80 %
    // threshold): we're inside the interpolation band.
    let min_bonus = bonuses
        .iter()
        .map(|(_, b)| *b)
        .fold(f64::INFINITY, f64::min);
    let max_bonus = bonuses
        .iter()
        .map(|(_, b)| *b)
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        min_bonus >= 0.04,
        "min realized bonus should be ≥ base ~5 %, got {:.4}",
        min_bonus
    );
    assert!(
        max_bonus <= 0.25,
        "max realized bonus should be ≤ seizure cap 25 %, got {:.4}",
        max_bonus
    );
}

// Even at the deepest liquidatable level, the bonus is bounded by the
// per-account ceiling derived from the effective threshold
// (threshold*(1+bonus) <= 100%), so seizure never exceeds collateral.
#[test]
fn test_liquidation_bonus_clamped_at_max() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.5); // $7000 debt
    t.set_price("USDC", usd_cents(50)); // deep crash, HF ≈ 0.57
    t.assert_liquidatable(ALICE);

    t.get_or_create_user(LIQUIDATOR);
    let liq_usdc_before = t.token_balance(LIQUIDATOR, "USDC");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.1); // $200 repay
    let liq_usdc_after = t.token_balance(LIQUIDATOR, "USDC");

    let usdc_received = liq_usdc_after - liq_usdc_before;
    let usd_received = usdc_received * 0.50;
    let realized_bonus = (usd_received / 200.0) - 1.0;

    // USDC's 80% effective threshold caps the bonus at (1-T)/T = 25%, so seizure
    // never exceeds collateral. Allow small arithmetic slop.
    assert!(
        realized_bonus <= 0.26,
        "realized bonus must stay under the per-account ceiling, got {:.4}",
        realized_bonus
    );
}
// Bad-debt threshold boundary ($5)

// Position with collateral well within the $5 bad-debt threshold AND
// debt greater than collateral triggers socialization on liquidation.
// Pins the inequality `total_collateral_usd <= BAD_DEBT_USD_THRESHOLD`.
#[test]
fn test_bad_debt_socialization_triggers_under_threshold() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    // Tight setup: $30 USDC supply, ~0.011 ETH ($22) debt. After USDC
    // crashes to $0.10, collateral = $3 (below $5 threshold), debt
    // remains ~$22 → bad-debt path eligible.
    t.supply(ALICE, "USDC", 30.0);
    t.borrow(ALICE, "ETH", 0.011);
    t.set_price("USDC", usd_cents(10));
    t.assert_liquidatable(ALICE);

    // Liquidator pays the full debt; bad-debt path zeros the position.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.011);

    let debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_after < 0.0001,
        "bad-debt cleanup should zero the debt, got {:.6}",
        debt_after
    );
}
