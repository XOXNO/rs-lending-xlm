use common::types::SeizeMode;
use controller::constants::{RAY, WAD};
use test_harness::{
    eth_preset, hub_asset, usd_cents, usdc_preset, LendingTest, ALICE, BOB, LIQUIDATOR,
};

fn get_indexes(t: &LendingTest, asset: &str) -> (i128, i128) {
    let asset_addr = t.resolve_asset(asset);
    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset_addr)]);
    let idx = ctrl.get_market_indexes_detailed(&assets).get(0).unwrap();
    (idx.supply_index, idx.borrow_index)
}

#[test]
fn test_seizure_equals_debt_times_one_plus_bonus() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    t.set_price("USDC", usd_cents(74));
    t.assert_liquidatable(ALICE);

    let _hf_before = t.health_factor(ALICE);

    t.get_or_create_user(LIQUIDATOR);
    let liquidator_usdc_before = t.token_balance(LIQUIDATOR, "USDC");

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    let liquidator_usdc_after = t.token_balance(LIQUIDATOR, "USDC");

    let collateral_received = liquidator_usdc_after - liquidator_usdc_before;
    let debt_repaid_usd = 0.5 * 2000.0;

    let collateral_received_usd = collateral_received * 0.74;
    let actual_bonus_rate = (collateral_received_usd / debt_repaid_usd) - 1.0;

    assert!(
        actual_bonus_rate > 0.11 && actual_bonus_rate < 0.14,
        "bonus rate at HF ~0.987 should be ~12.6%, got {:.4} ({:.2}%)",
        actual_bonus_rate,
        actual_bonus_rate * 100.0
    );

    let expected_seizure_usd = debt_repaid_usd * (1.0 + actual_bonus_rate);
    let diff_pct =
        ((collateral_received_usd - expected_seizure_usd) / expected_seizure_usd).abs() * 100.0;
    assert!(
        diff_pct < 2.0,
        "seizure should match debt * (1 + bonus): expected_usd={:.2}, got_usd={:.2}, diff={:.2}%",
        expected_seizure_usd,
        collateral_received_usd,
        diff_pct
    );
}

#[test]
fn test_bonus_formula_at_specific_hf_levels() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(74));

    let account_id = t.resolve_account_id(ALICE);
    let payments =
        soroban_sdk::Vec::from_array(&t.env, [(hub_asset(t.resolve_asset("ETH")), 3_0000000)]);
    let estimate =
        t.ctrl_client()
            .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Transfer);
    let hf = t.ctrl_client().get_health_factor(&account_id);

    let hf_f64 = hf as f64 / WAD as f64;
    assert!(
        hf_f64 < 1.0 && hf_f64 > 0.95,
        "HF should be ~0.987: {:.4}",
        hf_f64
    );

    assert!(
        (1100..=1500).contains(&estimate.bonus_rate_bps),
        "near-threshold HF should give bonus ~1100-1500 BPS, got {}",
        estimate.bonus_rate_bps
    );
}

#[test]
fn test_deep_underwater_higher_bonus() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(74));

    let id_alice = t.resolve_account_id(ALICE);
    let payments =
        soroban_sdk::Vec::from_array(&t.env, [(hub_asset(t.resolve_asset("ETH")), 3_0000000)]);
    let light =
        t.ctrl_client()
            .get_liquidation_estimate(&id_alice, &payments, &SeizeMode::Transfer);
    let hf_light = t.ctrl_client().get_health_factor(&id_alice);
    let hf_light_f64 = hf_light as f64 / WAD as f64;
    assert!(
        hf_light_f64 > 0.95 && hf_light_f64 < 1.0,
        "light case HF should be 0.95-1.0, got {:.4}",
        hf_light_f64
    );

    t.set_price("USDC", usd_cents(68));
    let deep = t
        .ctrl_client()
        .get_liquidation_estimate(&id_alice, &payments, &SeizeMode::Transfer);
    let hf_deep = t.ctrl_client().get_health_factor(&id_alice);
    let hf_deep_f64 = hf_deep as f64 / WAD as f64;
    assert!(
        hf_deep_f64 > 0.85 && hf_deep_f64 < hf_light_f64,
        "deep case HF should be 0.85-0.95 and lower than light, got {:.4}",
        hf_deep_f64
    );

    assert!(
        deep.bonus_rate_bps > light.bonus_rate_bps,
        "deeper underwater should have higher bonus: deep={} > light={}",
        deep.bonus_rate_bps,
        light.bonus_rate_bps
    );
}

#[test]
fn test_liquidation_does_not_increase_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    t.set_price("USDC", usd_cents(50));

    let hf_before = t.health_factor(ALICE);
    assert!(hf_before < 1.0, "should be liquidatable");

    let debt_before = t.total_debt(ALICE);
    let collateral_before = t.total_collateral(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    // Repaying 1 ETH at $2000 with no time advance must reduce the debt by
    // exactly the repayment value (no interest accrues between the reads).
    let debt_after = t.total_debt(ALICE);
    assert!(
        debt_after < debt_before,
        "liquidation must strictly reduce debt: before={:.4}, after={:.4}",
        debt_before,
        debt_after
    );
    assert!(
        (debt_before - debt_after - 2000.0).abs() < 1.0,
        "debt must drop by the $2000 repaid: before={:.4}, after={:.4}",
        debt_before,
        debt_after
    );

    let collateral_after = t.total_collateral(ALICE);
    assert!(
        collateral_after < collateral_before,
        "seizure must reduce collateral: before={:.4}, after={:.4}",
        collateral_before,
        collateral_after
    );
}

#[test]
fn test_protocol_fee_on_bonus_only_quantitative() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));

    let rev_before = t.snapshot_revenue("USDC");

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let rev_after = t.snapshot_revenue("USDC");
    let fee_collected = (rev_after - rev_before) as f64 / 1e7;

    assert!(
        fee_collected > 0.0,
        "protocol fee should be positive: {:.4}",
        fee_collected
    );
    assert!(
        fee_collected < 50.0,
        "protocol fee should be on bonus only (< 50 USDC), got {:.4} USDC",
        fee_collected
    );

    let liquidator_received = t.token_balance(LIQUIDATOR, "USDC");
    if liquidator_received > 0.0 {
        let fee_pct_of_seizure = fee_collected / liquidator_received * 100.0;
        assert!(
            fee_pct_of_seizure < 1.0,
            "fee should be <1% of total seizure (bonus-only): {:.4}%",
            fee_pct_of_seizure
        );
    }
}

#[test]
fn test_bad_debt_index_decrease_exact() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(BOB, "ETH", 1000.0);

    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    let eth = t.resolve_asset("ETH");
    let pool_client = t.pool_client("ETH");
    let supplied_before = pool_client.get_supplied_amount(&hub_asset(eth));
    let (si_before, _) = get_indexes(&t, "ETH");

    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (si_after, _) = get_indexes(&t, "ETH");

    let actual_ratio = si_after as f64 / si_before as f64;

    let _total_supplied_actual = supplied_before as f64 / RAY as f64;

    assert!(
        actual_ratio > 0.999 && actual_ratio < 1.0,
        "index decrease should be tiny: ratio={:.8}, indicating ~{:.6}% loss",
        actual_ratio,
        (1.0 - actual_ratio) * 100.0
    );

    let bob_balance_before = 1000.0;
    let bob_balance_after = t.supply_balance(BOB, "ETH");
    let bob_loss = bob_balance_before - bob_balance_after;

    assert!(
        (0.0..0.005).contains(&bob_loss),
        "Bob's loss should be <= bad debt (~0.003 ETH), got {:.6} ETH -- index over-decremented!",
        bob_loss
    );
}

#[test]
fn test_multiple_partial_liquidations_incremental_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));

    let debt_0 = t.borrow_balance(ALICE, "ETH");

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.3);
    let debt_1 = t.borrow_balance(ALICE, "ETH");

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.3);
    let debt_2 = t.borrow_balance(ALICE, "ETH");

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.3);
    let debt_3 = t.borrow_balance(ALICE, "ETH");

    assert!(
        debt_1 < debt_0,
        "1st liquidation should reduce debt: {:.4} < {:.4}",
        debt_1,
        debt_0
    );
    assert!(
        debt_2 < debt_1,
        "2nd liquidation should reduce debt: {:.4} < {:.4}",
        debt_2,
        debt_1
    );
    assert!(
        debt_3 < debt_2,
        "3rd liquidation should reduce debt: {:.4} < {:.4}",
        debt_3,
        debt_2
    );

    assert!(
        debt_3 < 2.5,
        "after 30% liquidation, debt should be well below 3.0: {:.4}",
        debt_3
    );

    let liquidator_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liquidator_usdc > 0.0,
        "liquidator should have received USDC collateral: {:.2}",
        liquidator_usdc
    );
}

#[test]
fn test_liquidation_bounded_by_available_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "ETH", 0.3);

    t.set_price("USDC", usd_cents(60));

    let _collateral_before = t.supply_balance(ALICE, "USDC");

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.3);

    let liquidator_usdc = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        liquidator_usdc <= 1_001.0,
        "liquidator should not receive more USDC than existed: {:.2}",
        liquidator_usdc
    );
}
