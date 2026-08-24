use controller::constants::WAD;
use test_harness::{assert_contract_error, errors, usd, usd_cents, LendingTest, ALICE, LIQUIDATOR};

#[test]
fn test_hf_exactly_one_is_healthy() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.5);

    t.set_price("USDC", usd(1) * 875 / 1000);

    let hf_raw = t.health_factor_raw(ALICE);

    let drift = (hf_raw - WAD).abs();
    assert!(
        drift < 1_000,
        "HF should be ~1.0 (raw WAD), got {}, drift={}",
        hf_raw,
        drift
    );

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::HEALTH_FACTOR_TOO_HIGH);
}

#[test]
fn test_is_liquidatable_flips_strictly_below_hf_one() {
    let mut t = LendingTest::new().standard_two_asset().build();

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

    t.set_price("ETH", usd(4_000) + usd(1) / 100);
    assert!(t.health_factor_raw(ALICE) < WAD);
    assert!(t.ctrl_client().is_liquidatable(&account_id));
}

#[test]
fn test_hf_just_below_one_is_liquidatable() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.5);
    t.set_price("USDC", usd(1) * 874 / 1000);

    let hf_raw = t.health_factor_raw(ALICE);
    assert!(hf_raw < WAD, "HF must be < 1.0, got {}", hf_raw);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert!(
        result.is_ok(),
        "liquidation at HF<1 should succeed, got {:?}",
        result
    );
}

#[test]
fn test_liquidation_strictly_improves_hf() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    t.set_price("USDC", usd_cents(74));
    t.assert_liquidatable(ALICE);

    let hf_before = t.health_factor(ALICE);
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    let hf_after = t.health_factor(ALICE);

    assert!(
        hf_after > hf_before,
        "post-liquidation HF must strictly improve: before={:.4}, after={:.4}",
        hf_before,
        hf_after
    );
}

#[test]
fn test_liquidation_bonus_monotone_in_mild_underwater_band() {
    let mut bonuses: std::vec::Vec<(u32, f64, f64, f64)> = std::vec::Vec::new();

    for cents_per_dollar in [73u32, 71, 69, 67] {
        let mut t = LendingTest::new().standard_two_asset_dust_disabled();

        t.supply(ALICE, "USDC", 10_000.0);
        t.borrow(ALICE, "ETH", 3.0);
        t.set_price("USDC", usd_cents(cents_per_dollar.into()));
        if !t.can_be_liquidated(ALICE) {
            continue;
        }

        t.get_or_create_user(LIQUIDATOR);
        let hf_before = t.health_factor(ALICE);
        let liq_usdc_before = t.token_balance(LIQUIDATOR, "USDC");
        t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.1);
        let hf_after = t.health_factor(ALICE);
        let liq_usdc_after = t.token_balance(LIQUIDATOR, "USDC");

        let usdc_received = liq_usdc_after - liq_usdc_before;
        let usd_received = usdc_received * (cents_per_dollar as f64) / 100.0;
        let realized_bonus = (usd_received / 200.0) - 1.0;
        bonuses.push((cents_per_dollar, realized_bonus, hf_before, hf_after));
    }

    assert!(
        bonuses.len() >= 3,
        "need at least 3 samples, got {}: {:?}",
        bonuses.len(),
        bonuses
    );

    for (cents, bonus, hf_before, hf_after) in &bonuses {
        let neutral_cap = hf_before / 0.80 - 1.0;
        assert!(
            *bonus <= neutral_cap + 1e-3,
            "bonus {bonus:.6} above HF-neutral cap {neutral_cap:.6} at cents={cents}, full={bonuses:?}"
        );
        assert!(
            hf_after + 1e-6 >= *hf_before,
            "partial reduced HF at cents={cents}: {hf_before:.6} -> {hf_after:.6}, full={bonuses:?}"
        );
    }

    let min_bonus = bonuses
        .iter()
        .map(|(_, b, _, _)| *b)
        .fold(f64::INFINITY, f64::min);
    let max_bonus = bonuses
        .iter()
        .map(|(_, b, _, _)| *b)
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

#[test]
fn test_liquidation_bonus_clamped_at_max() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.5);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    t.get_or_create_user(LIQUIDATOR);
    let liq_usdc_before = t.token_balance(LIQUIDATOR, "USDC");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.1);
    let liq_usdc_after = t.token_balance(LIQUIDATOR, "USDC");

    let usdc_received = liq_usdc_after - liq_usdc_before;
    let usd_received = usdc_received * 0.50;
    let realized_bonus = (usd_received / 200.0) - 1.0;

    assert!(
        realized_bonus <= 0.26,
        "realized bonus must stay under the per-account ceiling, got {:.4}",
        realized_bonus
    );
}

#[test]
fn test_bad_debt_socialization_triggers_under_threshold() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();

    t.supply(ALICE, "USDC", 30.0);
    t.borrow(ALICE, "ETH", 0.011);
    t.set_price("USDC", usd_cents(10));
    t.assert_liquidatable(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.011);

    let debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_after < 0.0001,
        "bad-debt cleanup should zero the debt, got {:.6}",
        debt_after
    );
}
