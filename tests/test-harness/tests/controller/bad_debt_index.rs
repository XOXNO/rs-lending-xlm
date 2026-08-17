use controller::constants::WAD;
use test_harness::{
    assert_contract_error, days, errors, hub_asset, usd, usd_cents, LendingTest, ALICE, BOB, CAROL,
    DAVE, LIQUIDATOR,
};

fn get_indexes(t: &LendingTest, asset: &str) -> (i128, i128) {
    let asset_addr = t.resolve_asset(asset);
    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset_addr)]);
    let idx = ctrl.get_market_indexes_detailed(&assets).get(0).unwrap();
    (idx.supply_index, idx.borrow_index)
}

fn setup() -> LendingTest {
    LendingTest::new().standard_two_asset_dust_disabled()
}

#[test]
fn test_bad_debt_decreases_supply_index() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    let (si_before, _) = get_indexes(&t, "ETH");

    t.set_price("USDC", usd_cents(10));
    t.assert_liquidatable(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (si_after, _) = get_indexes(&t, "ETH");

    assert!(
        si_after < si_before,
        "supply index should DECREASE after bad debt: before={}, after={}",
        si_before,
        si_after
    );

    let decrease_ratio = si_after as f64 / si_before as f64;
    assert!(
        decrease_ratio > 0.99 && decrease_ratio < 1.0,
        "decrease should be small relative to total supply: ratio={:.6}",
        decrease_ratio
    );
}

#[test]
fn test_bad_debt_loss_distributed_proportionally() {
    let mut t = setup();

    t.supply(BOB, "ETH", 75.0);
    t.supply(CAROL, "ETH", 25.0);

    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    let bob_before = t.supply_balance(BOB, "ETH");
    let carol_before = t.supply_balance(CAROL, "ETH");

    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let bob_after = t.supply_balance(BOB, "ETH");
    let carol_after = t.supply_balance(CAROL, "ETH");

    let bob_loss = bob_before - bob_after;
    let carol_loss = carol_before - carol_after;

    assert!(
        bob_loss > 0.0,
        "Bob should lose from bad debt: {:.6}",
        bob_loss
    );
    assert!(
        carol_loss > 0.0,
        "Carol should lose from bad debt: {:.6}",
        carol_loss
    );

    if carol_loss > 0.0001 {
        let ratio = bob_loss / carol_loss;
        assert!(
            (ratio - 3.0).abs() < 0.3,
            "loss should be proportional (3:1): ratio={:.4}, bob_loss={:.6}, carol_loss={:.6}",
            ratio,
            bob_loss,
            carol_loss
        );
    }
}

#[test]
fn test_bad_debt_index_floored_at_safety_floor() {
    let mut t = setup();

    t.supply(BOB, "ETH", 0.01);

    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.005);

    t.set_price("USDC", usd_cents(1));

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (si_after, _) = get_indexes(&t, "ETH");

    assert!(
        si_after >= controller::constants::SUPPLY_INDEX_FLOOR_RAW,
        "supply index should be floored at {}, got {}",
        controller::constants::SUPPLY_INDEX_FLOOR_RAW,
        si_after
    );
}

#[test]
fn test_supply_index_recovers_after_bad_debt() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (si_after_bad_debt, _) = get_indexes(&t, "ETH");

    t.set_price("USDC", usd(1));

    t.supply(DAVE, "USDC", 500_000.0);
    t.borrow(DAVE, "ETH", 30.0);

    t.advance_and_sync(days(365));

    let (si_recovered, _) = get_indexes(&t, "ETH");

    assert!(
        si_recovered > si_after_bad_debt,
        "supply index should recover with new interest: post_bad_debt={}, recovered={}",
        si_after_bad_debt,
        si_recovered
    );
}

#[test]
fn test_force_socialize_bad_debt_above_dust_threshold() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.02);

    t.set_price("USDC", usd_cents(30));
    let account_id = t.resolve_account_id(ALICE);

    let collateral = t.total_collateral_raw(ALICE);
    let debt = t.total_debt_raw(ALICE);
    assert!(
        collateral > 5 * WAD,
        "fixture must sit strictly above the $5 dust gate: collateral_wad={collateral}"
    );
    assert!(
        debt > collateral,
        "fixture must be insolvent: debt_wad={debt} collateral_wad={collateral}"
    );

    let refused = t.try_clean_bad_debt_by_id(account_id);
    assert_contract_error(refused, errors::CANNOT_CLEAN_BAD_DEBT);

    let (si_before, _) = get_indexes(&t, "ETH");

    t.force_socialize_bad_debt_by_id(account_id);

    let (si_after, _) = get_indexes(&t, "ETH");
    assert!(
        si_after < si_before,
        "force-socialize must drop the ETH supply index: before={si_before}, after={si_after}"
    );
    t.assert_no_positions(ALICE);
}

#[test]
fn test_force_socialize_rejects_healthy_account() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.01);

    let account_id = t.resolve_account_id(ALICE);
    let refused = t.try_force_socialize_bad_debt_by_id(account_id);
    assert_contract_error(refused, errors::CANNOT_CLEAN_BAD_DEBT);
}

#[test]
fn test_keeper_clean_bad_debt_decreases_supply_index() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 8.0);
    t.borrow(ALICE, "ETH", 0.002);

    let (si_before, _) = get_indexes(&t, "ETH");

    t.set_price("USDC", usd_cents(5));

    let account_id = t.resolve_account_id(ALICE);
    t.clean_bad_debt_by_id(account_id);

    let (si_after, _) = get_indexes(&t, "ETH");

    assert!(
        si_after < si_before,
        "keeper clean_bad_debt should decrease supply index: before={}, after={}",
        si_before,
        si_after
    );

    t.assert_no_positions(ALICE);
}

#[test]
fn test_bad_debt_does_not_affect_borrow_index() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    t.advance_and_sync(days(1));
    let (_, bi_before) = get_indexes(&t, "ETH");

    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (_, bi_after) = get_indexes(&t, "ETH");

    assert!(
        bi_after >= bi_before,
        "borrow index should never decrease, even during bad debt: before={}, after={}",
        bi_before,
        bi_after
    );
}

#[test]
fn test_bad_debt_reduction_matches_formula() {
    let mut t = setup();

    t.supply(BOB, "ETH", 1000.0);

    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    let bob_balance_before = t.supply_balance(BOB, "ETH");

    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let bob_balance_after = t.supply_balance(BOB, "ETH");
    let bob_loss = bob_balance_before - bob_balance_after;

    assert!(
        bob_loss > 0.0 && bob_loss < 0.01,
        "Bob's loss should be small (~ bad debt amount): {:.6} ETH",
        bob_loss
    );
}
