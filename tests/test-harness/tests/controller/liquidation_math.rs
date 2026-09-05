use crate::shared::get_indexes;
use common::types::SeizeMode;
use controller::constants::WAD;
use test_harness::{hub_asset, usd_cents, LendingTest, ALICE, BOB, LIQUIDATOR};

#[test]
fn test_seizure_equals_debt_times_one_plus_bonus() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    t.set_price("USDC", usd_cents(74));
    t.assert_liquidatable(ALICE);

    let _hf_before = t.health_factor(ALICE);

    t.get_or_create_user(LIQUIDATOR);
    let liquidator_usdc_before = t.token_balance(LIQUIDATOR, "USDC");

    // Independent source for the bonus: the estimate view reads the curve, not
    // the seizure we are about to measure. Deriving it from the seizure instead
    // makes `seizure == debt * (1 + bonus)` an identity that cannot fail.
    let account_id = t.resolve_account_id(ALICE);
    let payments =
        soroban_sdk::Vec::from_array(&t.env, [(hub_asset(t.resolve_asset("ETH")), 5_000_000)]);
    let bonus_bps = t
        .ctrl_client()
        .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Transfer)
        .bonus_rate_bps as f64;
    let fee_bps = f64::from(t.get_asset_config("USDC").liquidation_fees);

    let debt_before = t.borrow_balance(ALICE, "ETH");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    let liquidator_usdc_after = t.token_balance(LIQUIDATOR, "USDC");
    assert!(
        (debt_before - t.borrow_balance(ALICE, "ETH") - 0.5).abs() < 1e-6,
        "the whole 0.5 ETH must be repaid for the closed form to apply"
    );

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

    // Gross seizure is `debt * (1 + b)`; the protocol keeps `fee_bps` of the
    // bonus leg only, so the liquidator nets `debt * (1 + b * (1 - fee))`.
    let expected_seizure_usd =
        debt_repaid_usd * (1.0 + (bonus_bps / 10_000.0) * (1.0 - fee_bps / 10_000.0));
    assert!(
        (collateral_received_usd - expected_seizure_usd).abs() < 1e-5,
        "seizure must match debt * (1 + bonus) net of the bonus-only fee: \
         expected_usd={expected_seizure_usd:.9}, got_usd={collateral_received_usd:.9}, \
         bonus_bps={bonus_bps}, fee_bps={fee_bps}"
    );
}

#[test]
fn test_bonus_formula_at_specific_hf_levels() {
    let mut t = LendingTest::new().standard_two_asset().build();

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
    let mut t = LendingTest::new().standard_two_asset().build();

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
    let mut t = LendingTest::new().standard_two_asset().build();

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
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));

    // The old bound (`fee < 50 USDC`) was within ~2x of the true value, and the
    // discriminating ratio check hid behind `if liquidator_received > 0.0` on an
    // absolute balance. Close the form instead: the fee is `fee_bps` of the
    // bonus leg, and `seized = principal * (1 + b)`.
    let account_id = t.resolve_account_id(ALICE);
    let payments =
        soroban_sdk::Vec::from_array(&t.env, [(hub_asset(t.resolve_asset("ETH")), 1_0000000)]);
    let estimate =
        t.ctrl_client()
            .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Transfer);
    let seized = estimate.seized_collaterals.get_unchecked(0).amount;
    let bonus_bps = estimate.bonus_rate_bps;
    let fee_bps = i128::from(t.get_asset_config("USDC").liquidation_fees);
    assert!(
        seized > 0 && bonus_bps > 0 && fee_bps > 0,
        "estimate must be live: seized={seized}, bonus_bps={bonus_bps}, fee_bps={fee_bps}"
    );
    let expected_fee = (seized * bonus_bps / (10_000 + bonus_bps)) * fee_bps / 10_000;

    let rev_before = t.snapshot_revenue("USDC");

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let fee_collected = t.snapshot_revenue("USDC") - rev_before;
    assert!(
        (fee_collected - expected_fee).abs() <= 2,
        "protocol fee must be the bonus-only charge {expected_fee}, got {fee_collected} \
         (seized={seized}, bonus_bps={bonus_bps}, fee_bps={fee_bps})"
    );
    assert!(
        fee_collected < seized * fee_bps / 10_000,
        "fee {fee_collected} matches a charge on the gross seizure, not on the bonus"
    );
}

#[test]
fn test_bad_debt_index_decrease_exact() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();

    t.supply(BOB, "ETH", 1000.0);

    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    let (si_before, _) = get_indexes(&t, "ETH");
    let bob_balance_before = t.supply_balance(BOB, "ETH");

    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (si_after, _) = get_indexes(&t, "ETH");

    let actual_ratio = si_after as f64 / si_before as f64;

    assert!(
        actual_ratio < 1.0,
        "socialization must decrease the supply index: ratio={actual_ratio:.12}"
    );

    let bob_balance_after = t.supply_balance(BOB, "ETH");
    let bob_loss = bob_balance_before - bob_balance_after;
    assert!(
        bob_loss > 0.0,
        "Bob must absorb part of the loss: {bob_loss:.9}"
    );

    // "exact" means the index move and the balance move are the same event:
    // the old `(0.999, 1.0)` band plus `bob_loss in [0.0, 0.005)` admitted zero
    // loss and any write-down inside a 0.1 % window.
    let expected_ratio = 1.0 - bob_loss / bob_balance_before;
    assert!(
        (actual_ratio - expected_ratio).abs() < 1e-8,
        "index ratio {actual_ratio:.12} must equal 1 - loss/balance {expected_ratio:.12} \
         (loss={bob_loss:.9}, balance={bob_balance_before})"
    );
}

#[test]
fn test_multiple_partial_liquidations_incremental_hf() {
    let mut t = LendingTest::new().standard_two_asset().build();

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
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "ETH", 0.3);

    t.set_price("USDC", usd_cents(60));

    let collateral_before = t.supply_balance_raw(ALICE, "USDC");
    t.get_or_create_user(LIQUIDATOR);
    let liq_before = t.token_balance_raw(LIQUIDATOR, "USDC");

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.3);

    // The old assertion was an upper bound on an *absolute* balance and never
    // read `collateral_before`: a seizure of zero passed. Bound the deltas.
    let seized = collateral_before - t.supply_balance_raw(ALICE, "USDC");
    let received = t.token_balance_raw(LIQUIDATOR, "USDC") - liq_before;
    assert!(
        seized > 0 && seized <= collateral_before,
        "seizure must be positive and bounded by the position: seized={seized}, had={collateral_before}"
    );
    assert!(
        received > 0 && received <= seized,
        "liquidator cannot receive more than was seized: received={received}, seized={seized}"
    );
}
