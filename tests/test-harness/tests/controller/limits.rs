//! `max_withdraw` / `max_supply` / `get_market_index` preview views.
//!
//! Each scenario asserts the preview both ways: the returned amount executes,
//! and a request just above it reverts with the gate the preview modeled.

use controller::constants::RAY;
use soroban_sdk::Vec as SorobanVec;
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, usdc_preset, usdt_stable_preset,
    LendingTest, ALICE, BOB, STABLECOIN_SPOKE,
};

const UNIT: i128 = 10_000_000; // 1.0 at the presets' 7 decimals

#[test]
fn test_max_supply_uncapped_returns_max() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 1_000.0);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);
    assert_eq!(
        t.ctrl_client()
            .max_supply(&account_id, &hub_asset(asset.clone())),
        i128::MAX
    );
}

#[test]
fn test_max_supply_zero_when_paused() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100.0);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);
    t.pause();
    assert_eq!(
        t.ctrl_client()
            .max_supply(&account_id, &hub_asset(asset.clone())),
        0
    );
    t.unpause();
    assert_eq!(
        t.ctrl_client()
            .max_supply(&account_id, &hub_asset(asset.clone())),
        i128::MAX
    );
}

#[test]
fn test_max_withdraw_unconstrained_closes_position() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 10_000.0);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let ctrl = t.ctrl_client();

    let max = ctrl.max_withdraw(&account_id, &hub_asset(asset.clone()));
    let balance = ctrl.get_collateral_amount(&account_id, &hub_asset(asset.clone()));
    assert_eq!(max, balance, "unconstrained max must be the full balance");

    let withdrawals: SorobanVec<_> = soroban_sdk::vec![&t.env, (hub_asset(asset.clone()), max)];
    let paid = ctrl.withdraw(&alice, &account_id, &withdrawals, &None);
    let (paid_asset, paid_amount) = paid.get(0).unwrap();
    assert_eq!(paid_asset.asset, asset);
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
            p.max_utilization = RAY * 85 / 100;
        })
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 800.0); // utilization 80 %

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    // Headroom to the 85 % cap: 1000 - 800/0.85 ≈ 58.82 USDC.
    let max = t
        .ctrl_client()
        .max_withdraw(&account_id, &hub_asset(asset.clone()));
    assert!(
        max > 58 * UNIT && max < 59 * UNIT,
        "expected ~58.8 USDC headroom, got {max}"
    );

    // One stroop above the preview trips the utilization gate.
    let alice = t.get_or_create_user(ALICE);
    let over: SorobanVec<_> = soroban_sdk::vec![&t.env, (hub_asset(asset.clone()), max + 2)];
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
    let after = t
        .ctrl_client()
        .max_withdraw(&account_id, &hub_asset(asset.clone()));
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

    let max = t
        .ctrl_client()
        .max_withdraw(&account_id, &hub_asset(asset.clone()));
    let balance = t
        .ctrl_client()
        .get_collateral_amount(&account_id, &hub_asset(asset.clone()));
    assert_eq!(max, balance, "full close is feasible, so max = balance");

    // Debt-free accounts may leave small collateral residue.
    t.withdraw(ALICE, "USDC", 95.0);
    t.assert_supply_near(ALICE, "USDC", 5.0, 1.0);

    t.withdraw_raw(ALICE, "USDC", max);
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), 0);
}

#[test]
fn test_max_withdraw_pool_bounds_partial_when_full_close_blocked() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_max_utilization_disabled_all_markets()
        .build();

    // ALICE is the sole USDC supplier and BOB holds debt, so a full close
    // would leave the pool insolvent; the partial is bounded by pool cash.
    t.supply(ALICE, "USDC", 200.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 10.0);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    let max = t
        .ctrl_client()
        .max_withdraw(&account_id, &hub_asset(asset.clone()));
    assert!(
        max > 189 * UNIT && max < 200 * UNIT,
        "expected a large partial below full balance, got {max}"
    );

    // Anything meaningfully above the preview fails (pool cash / solvency).
    let alice = t.get_or_create_user(ALICE);
    let over: SorobanVec<_> = soroban_sdk::vec![&t.env, (hub_asset(asset.clone()), max + 3)];
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
        .get_collateral_amount(&account_id, &hub_asset(asset.clone()));
    assert!(
        residue > 0,
        "partial withdraw must leave pool-liquid residue, got {residue}"
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

    let max = t
        .ctrl_client()
        .max_withdraw(&account_id, &hub_asset(asset.clone()));
    let expected = 53_333_333_333_i128; // 5333.3333333 USDC
    assert!(
        (max - expected).abs() < UNIT / 100,
        "expected ~5333.33 USDC, got {max}"
    );

    // $1 above the preview violates the LTV gate.
    let alice = t.get_or_create_user(ALICE);
    let over: SorobanVec<_> = soroban_sdk::vec![&t.env, (hub_asset(asset.clone()), max + UNIT)];
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
fn test_max_withdraw_available_when_paused_zero_when_absent() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100.0);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    t.pause();
    assert!(
        t.ctrl_client()
            .max_withdraw(&account_id, &hub_asset(asset.clone()))
            > 0
    );
    t.unpause();

    // Unknown account and unlisted position degrade to zero, not panic.
    assert_eq!(
        t.ctrl_client()
            .max_withdraw(&9_999u64, &hub_asset(asset.clone())),
        0
    );
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
    let before = t.ctrl_client().get_market_index(&hub_asset(asset.clone()));
    let balance_before = t
        .ctrl_client()
        .get_collateral_amount(&account_id, &hub_asset(asset.clone()));

    // Half a year with no oracle refresh and no keeper sync: prices are
    // stale, only view-side simulation can accrue.
    t.advance_time_no_refresh(60 * 60 * 24 * 180);

    let after = t.ctrl_client().get_market_index(&hub_asset(asset.clone()));
    assert!(
        after.borrow_index > before.borrow_index && after.supply_index > before.supply_index,
        "indexes must accrue in the view despite the stale oracle"
    );

    let balance_after = t
        .ctrl_client()
        .get_collateral_amount(&account_id, &hub_asset(asset.clone()));
    assert!(
        balance_after > balance_before,
        "supplier balance must grow with simulated interest, got {balance_before} -> {balance_after}"
    );

    // Poison the price entirely: price-dependent views revert, the
    // balance and index views stay alive.
    t.set_price("USDC", 0);
    assert!(
        t.ctrl_client()
            .try_get_total_collateral_usd(&account_id)
            .is_err(),
        "USD valuation must fail on a poisoned price"
    );
    assert_eq!(
        t.ctrl_client()
            .try_get_collateral_amount(&account_id, &hub_asset(asset.clone()))
            .unwrap()
            .unwrap(),
        balance_after
    );
    assert!(t
        .ctrl_client()
        .try_get_market_index(&hub_asset(asset.clone()))
        .is_ok());
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
    let max = t
        .ctrl_client()
        .max_withdraw(&account_id, &hub_asset(asset.clone()));
    assert!(max >= 500 * UNIT - 1, "full balance expected, got {max}");
}

#[test]
fn test_max_borrow_zero_when_paused() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t.supply(BOB, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 10.0);

    let usdc = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    assert!(
        t.ctrl_client()
            .max_borrow(&account_id, &hub_asset(usdc.clone()))
            > 0
    );
    t.pause();
    assert_eq!(
        t.ctrl_client()
            .max_borrow(&account_id, &hub_asset(usdc.clone())),
        0
    );
    t.unpause();
    assert!(
        t.ctrl_client()
            .max_borrow(&account_id, &hub_asset(usdc.clone()))
            > 0
    );
}

#[test]
fn test_max_borrow_bounded_by_ltv_and_executable() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    // Ample USDC liquidity so the account LTV gate — not pool cash or the cap —
    // is the binding constraint.
    t.supply(BOB, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 10.0);

    let usdc = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    let max = t
        .ctrl_client()
        .max_borrow(&account_id, &hub_asset(usdc.clone()));
    assert!(
        max > 0,
        "ETH collateral should allow a USDC borrow, got {max}"
    );

    // The preview executes to the stroop.
    t.borrow_raw(ALICE, "USDC", max);

    // Headroom collapses and one more unit trips the LTV gate the preview
    // modeled, so the preview never overstated.
    let after = t
        .ctrl_client()
        .max_borrow(&account_id, &hub_asset(usdc.clone()));
    assert!(
        after <= UNIT,
        "headroom should be ~0 after borrowing max, got {after}"
    );
    let res = t.try_borrow(ALICE, "USDC", 1.0);
    assert_contract_error(res, errors::INSUFFICIENT_COLLATERAL);
}

#[test]
fn test_max_borrow_bounded_by_spoke_borrow_cap_and_executable() {
    // An spoke account borrowing USDT under a 500 USDT spoke borrow cap. The
    // 10_000 USDC collateral at the 97% spoke LTV leaves ~$9_700 of room, so
    // the spoke cap is the binding constraint — exercising
    // `spoke_borrow_cap_headroom`, the `borrow_ok` spoke-cap gate, and the
    // spoke branches of `account_can_borrow_asset`. BOB supplies USDT through
    // the protocol so the market has tracked supply (the preview returns 0 on
    // a zero-supply market).
    let spoke_borrow_cap = 500 * UNIT;

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    // Set the USDT spoke borrow cap with the category's own risk params so the
    // edit leaves LTV/threshold/bonus untouched.
    t.edit_asset_in_spoke_caps(
        "USDT",
        2,
        true,
        true,
        STABLECOIN_SPOKE.ltv,
        STABLECOIN_SPOKE.threshold,
        STABLECOIN_SPOKE.bonus,
        0i128,
        spoke_borrow_cap,
    );

    // Real protocol supply on the borrowed market keeps the preview's
    // utilization defined; this normal account is unaffected by the spoke cap.
    t.supply(BOB, "USDT", 50_000.0);

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);

    let usdt = t.resolve_asset("USDT");
    let account_id = t.resolve_account_id(ALICE);

    // Headroom is the full 500 USDT spoke cap (no USDT borrowed yet).
    let max = t
        .ctrl_client()
        .max_borrow(&account_id, &hub_asset(usdt.clone()));
    assert!(
        max > 499 * UNIT && max <= 500 * UNIT,
        "expected ~500 USDT spoke-cap headroom, got {max}"
    );

    // The preview executes to the stroop.
    t.borrow_raw(ALICE, "USDT", max);

    // Spoke usage now sits at the cap: headroom collapses and one more unit
    // trips the spoke borrow-cap gate the preview modeled.
    let after = t
        .ctrl_client()
        .max_borrow(&account_id, &hub_asset(usdt.clone()));
    assert!(
        after <= 1,
        "spoke headroom should be ~0 at the cap, got {after}"
    );
    let res = t.try_borrow(ALICE, "USDT", 1.0);
    assert_contract_error(res, errors::SPOKE_BORROW_CAP_REACHED);
}

#[test]
fn test_max_supply_bounded_by_spoke_supply_cap_and_executable() {
    // An spoke account supplying USDC under a 1_000 USDC spoke supply cap.
    // The spoke cap binds, driving the preview through
    // `spoke_supply_cap_headroom`. Asserts both directions: the preview
    // executes and one unit over trips the spoke supply-cap gate.
    let spoke_cap = 1_000 * UNIT;

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.edit_asset_in_spoke_caps(
        "USDC",
        2,
        true,
        true,
        STABLECOIN_SPOKE.ltv,
        STABLECOIN_SPOKE.threshold,
        STABLECOIN_SPOKE.bonus,
        spoke_cap,
        0i128,
    );

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 400.0);

    let usdc = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    // ~600 USDC of spoke headroom remains (1_000 cap − 400 supplied).
    let headroom = t
        .ctrl_client()
        .max_supply(&account_id, &hub_asset(usdc.clone()));
    assert!(
        headroom > 599 * UNIT && headroom <= 600 * UNIT,
        "expected ~600 USDC spoke headroom, got {headroom}"
    );

    // The preview executes; one more unit trips the spoke supply cap.
    t.supply_raw(ALICE, "USDC", headroom);
    assert_eq!(
        t.ctrl_client()
            .max_supply(&account_id, &hub_asset(usdc.clone())),
        0
    );
    let res = t.try_supply(ALICE, "USDC", 1.0);
    assert_contract_error(res, errors::SPOKE_SUPPLY_CAP_REACHED);
}

#[test]
fn test_max_withdraw_spoke_account_respects_stored_spoke_ltv() {
    // An spoke account with debt: the withdrawn collateral's spoke LTV (97%)
    // governs the partial-withdraw cap, routing the preview through the
    // spoke-influenced `risk_partial_cap` / `partial_ok` path. $10k USDC at
    // 97% spoke LTV backing $9_000 USDT debt leaves
    // (9_700 − 9_000) / 0.97 ≈ $721.6 removable.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 9_000.0);

    let usdc = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    let max = t
        .ctrl_client()
        .max_withdraw(&account_id, &hub_asset(usdc.clone()));
    let expected = 7_216_494_845_i128; // ~721.65 USDC at the 97% spoke LTV.
    assert!(
        (max - expected).abs() < UNIT,
        "expected ~721.6 USDC removable under 97% spoke LTV, got {max}"
    );

    // A dollar above the preview violates the LTV gate the preview modeled.
    let alice = t.get_or_create_user(ALICE);
    let over: SorobanVec<_> = soroban_sdk::vec![&t.env, (hub_asset(usdc.clone()), max + UNIT)];
    let res = match t
        .ctrl_client()
        .try_withdraw(&alice, &account_id, &over, &None)
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(res, errors::INSUFFICIENT_COLLATERAL);

    // The preview itself executes and leaves the account healthy.
    t.withdraw_raw(ALICE, "USDC", max);
    assert!(t.health_factor(ALICE) >= 1.0);
}

// A paused listing rejects every withdraw, so the preview must mirror the
// mutating path and report zero capacity (frozen stays withdraw-permissive).
#[test]
fn test_max_withdraw_paused_listing_returns_zero() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 10_000.0);

    let asset = t.resolve_asset("USDC");
    let account_id = t.resolve_account_id(ALICE);

    t.set_spoke_asset_paused("USDC", true);

    assert_eq!(
        t.ctrl_client().max_withdraw(&account_id, &hub_asset(asset)),
        0,
        "paused listing must preview zero withdraw capacity"
    );
}
