//! `max_withdraw` / `max_supply` / `get_market_index` preview views.
//!
//! Each scenario asserts the preview both ways: the returned amount executes,
//! and a request just above it reverts with the gate the preview modeled.

use common::constants::RAY;
use soroban_sdk::Vec as SorobanVec;
use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, LendingTest, ALICE, BOB,
};

const UNIT: i128 = 10_000_000; // 1.0 at the presets' 7 decimals

#[test]
fn test_max_supply_uncapped_returns_max() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 1_000.0);

    let asset = t.resolve_asset("USDC");
    assert_eq!(t.ctrl_client().max_supply(&asset), i128::MAX);
}

#[test]
fn test_max_supply_tracks_cap_headroom_and_is_executable() {
    let mut preset = usdc_preset();
    preset.config.supply_cap = 2_000 * UNIT;
    let mut t = LendingTest::new().with_market(preset).build();

    t.supply(ALICE, "USDC", 500.0);

    let asset = t.resolve_asset("USDC");
    let headroom = t.ctrl_client().max_supply(&asset);
    assert!(
        headroom > 1_499 * UNIT && headroom <= 1_500 * UNIT,
        "headroom should be ~1500 USDC, got {headroom}"
    );

    // The preview executes; one more unit trips the cap.
    t.supply_raw(ALICE, "USDC", headroom);
    assert_eq!(t.ctrl_client().max_supply(&asset), 0);
    let res = t.try_supply(ALICE, "USDC", 1.0);
    assert_contract_error(res, errors::SUPPLY_CAP_REACHED);
}

#[test]
fn test_max_supply_zero_when_paused() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100.0);

    let asset = t.resolve_asset("USDC");
    t.pause();
    assert_eq!(t.ctrl_client().max_supply(&asset), 0);
    t.unpause();
    assert_eq!(t.ctrl_client().max_supply(&asset), i128::MAX);
}

#[test]
fn test_max_withdraw_unconstrained_closes_position() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 10_000.0);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let ctrl = t.ctrl_client();

    let max = ctrl.max_withdraw(&account_id, &asset);
    let balance = ctrl.collateral_amount_for_token(&account_id, &asset);
    assert_eq!(max, balance, "unconstrained max must be the full balance");

    let withdrawals: SorobanVec<_> = soroban_sdk::vec![&t.env, (asset.clone(), max)];
    let paid = ctrl.withdraw(&alice, &account_id, &withdrawals, &None);
    let (paid_asset, paid_amount) = paid.get(0).unwrap();
    assert_eq!(paid_asset, asset);
    // Full close pays the floor rounding of the half-up valuation.
    assert!(
        paid_amount == max || paid_amount == max - 1,
        "full close pays max or max-1 stroop, got {paid_amount} vs {max}"
    );
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), 0);
}

#[test]
fn test_max_withdraw_bounded_by_utilization_and_executable() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_params("USDC", |p| {
            p.max_utilization_ray = RAY * 85 / 100;
        })
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 800.0); // utilization 80 %

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    // Headroom to the 85 % cap: 1000 - 800/0.85 ≈ 58.82 USDC.
    let max = t.ctrl_client().max_withdraw(&account_id, &asset);
    assert!(
        max > 58 * UNIT && max < 59 * UNIT,
        "expected ~58.8 USDC headroom, got {max}"
    );

    // One stroop above the preview trips the utilization gate.
    let alice = t.get_or_create_user(ALICE);
    let over: SorobanVec<_> = soroban_sdk::vec![&t.env, (asset.clone(), max + 2)];
    let res = match t
        .ctrl_client()
        .try_withdraw(&alice, &account_id, &over, &None)
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(res, errors::UTILIZATION_ABOVE_MAX);

    // The preview itself executes.
    t.withdraw_raw(ALICE, "USDC", max);
    let after = t.ctrl_client().max_withdraw(&account_id, &asset);
    assert!(after <= 1, "pool sits at the cap, got {after}");
}

#[test]
fn test_max_withdraw_prefers_full_close_over_dusty_partial() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // CAROL's supply keeps the pool liquid, so ALICE's full close is
    // feasible and the preview returns the whole balance even though a
    // near-full partial would trip the $10 residue floor.
    t.supply(ALICE, "USDC", 100.0);
    t.supply(test_harness::CAROL, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 50.0);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    let max = t.ctrl_client().max_withdraw(&account_id, &asset);
    let balance = t
        .ctrl_client()
        .collateral_amount_for_token(&account_id, &asset);
    assert_eq!(max, balance, "full close is feasible, so max = balance");

    // A partial leaving a $5 residue is exactly what the preview avoids.
    let res = t.try_withdraw(ALICE, "USDC", 95.0);
    assert_contract_error(res, errors::DUST_RESIDUE_NOT_ALLOWED);

    t.withdraw_raw(ALICE, "USDC", max);
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), 0);
}

#[test]
fn test_max_withdraw_dust_floor_bounds_partial_when_full_close_blocked() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_max_utilization_disabled_all_markets()
        .build();

    // ALICE is the sole USDC supplier and BOB holds debt, so a full close
    // would leave the pool insolvent; the partial is bounded by the $10
    // residue floor (one stroop inside the cash bound).
    t.supply(ALICE, "USDC", 200.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 10.0);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    let max = t.ctrl_client().max_withdraw(&account_id, &asset);
    assert!(
        max > 189 * UNIT && max <= 190 * UNIT,
        "expected ~190 USDC (dust-floor bound), got {max}"
    );

    // Anything meaningfully above the preview fails (cash or dust gate).
    let alice = t.get_or_create_user(ALICE);
    let over: SorobanVec<_> = soroban_sdk::vec![&t.env, (asset.clone(), max + 3)];
    let res = match t
        .ctrl_client()
        .try_withdraw(&alice, &account_id, &over, &None)
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert!(res.is_err(), "max + 3 must not be withdrawable");

    t.withdraw_raw(ALICE, "USDC", max);
    let residue = t
        .ctrl_client()
        .collateral_amount_for_token(&account_id, &asset);
    assert!(
        residue >= 10 * UNIT,
        "residue must stay at or above the $10 floor, got {residue}"
    );
}

#[test]
fn test_max_withdraw_with_debt_respects_ltv() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // $10k collateral at 75 % LTV, $3.5k debt → removable value is
    // (7500 - 3500) / 0.75 ≈ $5333.33 (tighter than the 80 % HF gate).
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.75);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    let max = t.ctrl_client().max_withdraw(&account_id, &asset);
    let expected = 53_333_333_333_i128; // 5333.3333333 USDC
    assert!(
        (max - expected).abs() < UNIT / 100,
        "expected ~5333.33 USDC, got {max}"
    );

    // $1 above the preview violates the LTV gate.
    let alice = t.get_or_create_user(ALICE);
    let over: SorobanVec<_> = soroban_sdk::vec![&t.env, (asset.clone(), max + UNIT)];
    let res = match t
        .ctrl_client()
        .try_withdraw(&alice, &account_id, &over, &None)
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(res, errors::INSUFFICIENT_COLLATERAL);

    t.withdraw_raw(ALICE, "USDC", max);
    assert!(t.health_factor(ALICE) >= 1.0);
}

#[test]
fn test_max_withdraw_zero_when_paused_or_absent() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100.0);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    t.pause();
    assert_eq!(t.ctrl_client().max_withdraw(&account_id, &asset), 0);
    t.unpause();

    // Unknown account and unlisted position degrade to zero, not panic.
    assert_eq!(t.ctrl_client().max_withdraw(&9_999u64, &asset), 0);
}

#[test]
fn test_get_market_index_and_balance_views_survive_oracle_outage() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 500.0);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);
    let before = t.ctrl_client().get_market_index(&asset);
    let balance_before = t
        .ctrl_client()
        .collateral_amount_for_token(&account_id, &asset);

    // Half a year with no oracle refresh and no keeper sync: prices are
    // stale, only view-side simulation can accrue.
    t.advance_time_no_refresh(60 * 60 * 24 * 180);

    let after = t.ctrl_client().get_market_index(&asset);
    assert!(
        after.borrow_index_ray > before.borrow_index_ray
            && after.supply_index_ray > before.supply_index_ray,
        "indexes must accrue in the view despite the stale oracle"
    );

    let balance_after = t
        .ctrl_client()
        .collateral_amount_for_token(&account_id, &asset);
    assert!(
        balance_after > balance_before,
        "supplier balance must grow with simulated interest, got {balance_before} -> {balance_after}"
    );

    // Poison the price entirely: price-dependent views revert, the
    // balance and index views stay alive.
    t.set_price("USDC", 0);
    assert!(
        t.ctrl_client()
            .try_total_collateral_in_usd(&account_id)
            .is_err(),
        "USD valuation must fail on a poisoned price"
    );
    assert_eq!(
        t.ctrl_client()
            .try_collateral_amount_for_token(&account_id, &asset)
            .unwrap()
            .unwrap(),
        balance_after
    );
    assert!(t.ctrl_client().try_get_market_index(&asset).is_ok());
}

#[test]
fn test_max_withdraw_full_close_stays_price_free_for_debt_free_account() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 500.0);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    // With the price poisoned, the debt-free full-close preview still
    // resolves: it needs no oracle.
    t.set_price("USDC", 0);
    let max = t.ctrl_client().max_withdraw(&account_id, &asset);
    assert!(max >= 500 * UNIT - 1, "full balance expected, got {max}");
}
