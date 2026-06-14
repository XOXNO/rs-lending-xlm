use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usd_cents, usdc_preset, usdt_stable_preset,
    LendingTest, ALICE, LIQUIDATOR,
};
// Open-time gate

#[test]
fn test_supply_below_dust_floor_rejected() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    // $5 deposit < $10 floor → reject.
    let res = t.try_supply(ALICE, "USDC", 5.0);
    assert_contract_error(res, errors::DUST_RESIDUE_NOT_ALLOWED);
}

#[test]
fn test_supply_above_dust_floor_succeeds() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100.0); // $100 — well above floor.
    t.assert_supply_near(ALICE, "USDC", 100.0, 1.0);
}

// Regression: a pre-existing supply position whose USD value drifted under
// the dust floor due to a price crash (not user action) must NOT block an
// unrelated supply of a different, healthy asset. The dust gate on supply
// only applies to the assets actually being supplied in this action.
#[test]
fn test_supply_unrelated_asset_not_blocked_by_price_crashed_position() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies $15 USDC — above the $10 floor at open time.
    t.supply(ALICE, "USDC", 15.0);

    // USDC crashes to $0.50. Alice's position is now $7.50 < $10 floor.
    // She did not cause this — the market moved.
    t.set_price("USDC", controller::constants::WAD / 2);

    // Alice now deposits a healthy unrelated asset ($100 ETH). The
    // pre-existing crashed USDC position must not block this.
    let result = t.try_supply(ALICE, "ETH", 0.05); // 0.05 ETH @ $2000 = $100
    assert!(
        result.is_ok(),
        "supply of healthy unrelated asset must not be blocked by a \
         pre-existing position that drifted below dust floor; got {:?}",
        result
    );

    t.assert_supply_near(ALICE, "ETH", 0.05, 0.001);
}

// Regression: borrow must not be blocked by an unrelated supply position
// whose USD value drifted sub-floor from a price crash. Borrow only mutates
// borrow positions; the dust gate scopes to the touched (borrowed) assets.
#[test]
fn test_borrow_not_blocked_by_price_crashed_supply_position() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(usdt_stable_preset())
        .build();

    // Alice's collateral: $1000 ETH plus a small $15 USDC stake. Both
    // initially above the $10 floor.
    t.supply_bulk(ALICE, &[("ETH", 0.5), ("USDC", 15.0)]);

    // USDC crashes to $0.50 → USDC supply position now worth $7.50, dust.
    // ETH collateral untouched ($1000) so the account is still healthy.
    t.set_price("USDC", controller::constants::WAD / 2);

    // Alice borrows a healthy amount of USDT, well above the floor.
    let result = t.try_borrow(ALICE, "USDT", 50.0);
    assert!(
        result.is_ok(),
        "borrow must not be blocked by a pre-existing supply position \
         that drifted below dust floor; got {:?}",
        result
    );
}

// Regression: withdraw of a healthy position must not be blocked by an
// unrelated supply position that drifted sub-floor from a price crash.
// Withdraw mutates supply only; dust scopes to withdrawn assets.
#[test]
fn test_withdraw_not_blocked_by_price_crashed_other_supply_position() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply_bulk(ALICE, &[("ETH", 0.5), ("USDC", 15.0)]);

    // USDC crashes — USDC supply position now sub-floor at $7.50.
    t.set_price("USDC", controller::constants::WAD / 2);

    // Alice withdraws part of her ETH (leaves $900 of ETH, still well
    // above floor). The drifted USDC leg must not block this.
    let result = t.try_withdraw(ALICE, "ETH", 0.05);
    assert!(
        result.is_ok(),
        "withdraw of healthy position must not be blocked by an unrelated \
         pre-existing position that drifted below dust floor; got {:?}",
        result
    );
}

// Regression: repay must not be blocked by an unrelated borrow position
// whose USD value drifted sub-floor. Repay mutates borrow only; dust
// scopes to repaid assets.
#[test]
fn test_repay_not_blocked_by_price_crashed_other_borrow_position() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(usdt_stable_preset())
        .build();

    // Alice: $10k USDC supply, borrows two debts.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 100.0);
    t.borrow(ALICE, "ETH", 0.025); // $50 debt @ $2000

    // ETH crashes hard. Alice's ETH debt is now 0.025 × $40 = $1, dust.
    // (Her supply ETH/USDC isn't relevant — USDC is the collateral.)
    t.set_price("ETH", controller::constants::WAD * 40);

    // Alice repays $50 of USDT. ETH debt is untouched; its dust state is
    // not the repay path's concern.
    let result = t.try_repay(ALICE, "USDT", 50.0);
    assert!(
        result.is_ok(),
        "repay must not be blocked by an unrelated borrow position that \
         drifted below dust floor; got {:?}",
        result
    );
}
// Repay / withdraw partial-residue gates

#[test]
fn test_partial_repay_leaving_dust_debt_rejected() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies $10k, borrows $50 in ETH (0.025 ETH @ $2000).
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 0.025);

    // Repay $48 worth (0.024 ETH) → leaves $2 debt < $10 floor. Reject.
    let res = t.try_repay(ALICE, "ETH", 0.024);
    assert_contract_error(res, errors::DUST_RESIDUE_NOT_ALLOWED);
}

#[test]
fn test_full_repay_closes_position() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 0.025);
    // Full close — scaled debt goes to zero, dust gate skips.
    t.repay(ALICE, "ETH", 0.025);
}

#[test]
fn test_withdraw_leaving_dust_collateral_rejected() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100.0);
    // Withdraw $95 → leaves $5 collateral < $10 floor. Reject.
    let res = t.try_withdraw(ALICE, "USDC", 95.0);
    assert_contract_error(res, errors::DUST_RESIDUE_NOT_ALLOWED);
}

#[test]
fn test_withdraw_all_closes_position() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100.0);
    // `withdraw_all` is the closure helper — burns full position.
    t.withdraw_all(ALICE, "USDC");
}
// Liquidation debt-dust caps

#[test]
fn test_liquidation_caps_debt_dust_without_over_closing() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies $200 USDC and borrows $130 ETH (0.065 ETH @ $2000).
    // LTV is 0.75 → max borrow = $150; she's near the cap but healthy.
    t.supply(ALICE, "USDC", 200.0);
    t.borrow(ALICE, "ETH", 0.065);

    // Crash USDC to make Alice liquidatable. New collateral $100, debt
    // $130, HF = $100 * 0.80 / $130 = 0.615 → underwater.
    t.set_price("USDC", controller::constants::WAD / 2);
    t.assert_liquidatable(ALICE);

    // Liquidator repays $125 (just enough to push HF back near 1). The debt
    // dust cap must not force liquidation past the normal close amount.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.0625);

    // The call succeeds without leaving a nonzero sub-floor debt residue.
}

// Regression for Codex adversarial-review #1: debt-dust handling may never
// raise the seizure target beyond what the liquidator has actually paid.
// A liquidator who supplies a partial payment must receive collateral
// scaled to *that* payment — never collateral scaled to full debt.
#[test]
fn test_liquidation_partial_payment_does_not_over_seize_on_dust_cap() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies $200 USDC, borrows $130 ETH.
    t.supply(ALICE, "USDC", 200.0);
    t.borrow(ALICE, "ETH", 0.065);
    t.set_price("USDC", controller::constants::WAD / 2);
    t.assert_liquidatable(ALICE);

    // Snapshot Alice's debt before. Under the over-seize bug, dust handling
    // would zero out her debt position based on a partial payment. Under the
    // fix, debt drops by ~payment value (not full).
    let debt_before = t.borrow_balance(ALICE, "ETH");

    // Liquidator submits a deliberately small payment (~$2 ETH = 0.001
    // ETH). Under the bug this would seize against full debt (~$130 worth of
    // USDC collateral); under the fix the repay target is bounded by the paid
    // value and any debt-dust adjustment can only cap/refund repayment.
    let _ = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let debt_after = t.borrow_balance(ALICE, "ETH");
    let debt_reduction = debt_before - debt_after;

    // The liquidator paid 0.001 ETH (~$2). Debt should drop by at most that
    // same amount. If debt drops by anything close to the full ~0.065 ETH the
    // over-seize bug is back.
    assert!(
        debt_reduction < 0.005,
        "debt dropped by {:.4} ETH on a 0.001 ETH partial payment — \
         dust cap is over-seizing (Codex #1 regression)",
        debt_reduction
    );
}

// A liquidator whose payment would leave the repaid debt leg below its min debt
// floor, but still does not cover that leg fully, must not make liquidation
// revert. The engine caps that leg's repayment to leave at least the asset's
// debt floor and refunds the excess.
//
// Setup: collateral barely above debt × (1 + base_bonus). The math
// engine's `d_max = collateral / (1 + base_bonus)` lands within $10 of
// `total_debt`, so the optimal partial repayment would leave dust. With a
// payment short of full debt, the fix caps the debt leg instead of rejecting.
#[test]
fn test_liquidation_partial_above_optimal_clamps_when_residue_would_be_dust() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice: $30 USDC collateral, 0.012 ETH debt ($24). Healthy at supply.
    // $30 USDC collateral × 0.75 LTV → max borrow $22.50. 0.011 ETH =
    // $22 stays under that ceiling.
    t.supply(ALICE, "USDC", 30.0);
    t.borrow(ALICE, "ETH", 0.011);

    // Halve USDC. Collateral → $15, debt → $24, HF = $15·0.80/$24 ≈ 0.50.
    // d_max ≈ $15/1.05 ≈ $14.29 leaves residue ≈ $9.71 — sub-$10 floor.
    t.set_price("USDC", controller::constants::WAD / 2);
    t.assert_liquidatable(ALICE);

    // Pay 0.010 ETH = $20 — above the optimal partial target (≈ $14)
    // but $2 short of total debt ($22). The engine clamps the close amount
    // rather than reverting.
    let debt_before = t.borrow_balance(ALICE, "ETH");
    t.get_or_create_user(LIQUIDATOR);
    let liquidator_usdc_before = t.token_balance(LIQUIDATOR, "USDC");
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.010);
    assert!(
        result.is_ok(),
        "liquidation should clamp instead of reverting"
    );
    let debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(debt_after < debt_before);
    assert!(
        debt_after < 0.0001 || debt_after >= 0.0049,
        "ETH debt residue should be zero or stay near/above the $10 floor, got {} ETH",
        debt_after
    );
    let liquidator_usdc_after = t.token_balance(LIQUIDATOR, "USDC");
    let seized_usdc = liquidator_usdc_after - liquidator_usdc_before;
    assert!(
        seized_usdc < 27.0,
        "seizure must be sized from the capped repayment, not the pre-cap close amount; got {seized_usdc} USDC"
    );
}

// Multi-asset companion: a user with multiple debt positions where one touched
// debt position's residue would land sub-floor must still remain liquidatable,
// but the touched leg must be capped so it does not end below its own floor.
#[test]
fn test_liquidation_caps_per_position_dust_on_multi_debt_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(usdt_stable_preset())
        .build();

    // Alice supplies $400 USDC, borrows two debts:
    //   - 0.011 ETH ≈ $22 (a small leg that can fall into the dust window)
    //   - 100 USDT  ≈ $100 (a large leg that will keep the aggregate
    //                       residue well above $10)
    t.supply(ALICE, "USDC", 400.0);
    t.borrow(ALICE, "ETH", 0.011);
    t.borrow(ALICE, "USDT", 100.0);

    // Crash USDC to push Alice underwater.
    t.set_price("USDC", controller::constants::WAD * 35 / 100);
    t.assert_liquidatable(ALICE);

    // Liquidator pays only on the small ETH leg, just enough to leave
    // ~$1–2 ETH dust. The USDT leg is untouched (still ~$100), so the account
    // should not be protected from liquidation by the small-leg residue.
    let debt_before = t.borrow_balance(ALICE, "ETH");
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.010);
    assert!(result.is_ok(), "liquidation should remain live");
    let eth_debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(eth_debt_after < debt_before);
    assert!(
        eth_debt_after < 0.0001 || eth_debt_after >= 0.0049,
        "ETH debt residue should be zero or stay near/above the $10 floor, got {} ETH",
        eth_debt_after
    );
}

// Companion: when the same setup gets a payment that fully covers total debt,
// liquidation still settles without leaving a nonzero sub-floor debt residue.
#[test]
fn test_liquidation_full_payment_closes_dust_position() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // $30 USDC collateral × 0.75 LTV → max borrow $22.50. 0.011 ETH =
    // $22 stays under that ceiling.
    t.supply(ALICE, "USDC", 30.0);
    t.borrow(ALICE, "ETH", 0.011);
    t.set_price("USDC", controller::constants::WAD / 2);
    t.assert_liquidatable(ALICE);

    // Pay 0.011 ETH = $22 = total debt.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.011);
    let debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_after < 0.0001,
        "full payment should close the position, got {} ETH residual",
        debt_after
    );
}

// Regression: if a debt position drifted wholly below its debt floor and the
// liquidator offers enough to close it, the close-factor refund pass must not
// shrink that leg below full close. Otherwise the dust cap drops the only repay
// leg, liquidation reverts as empty, and collateral above the bad-debt cleanup
// threshold leaves the account stuck.
#[test]
fn test_liquidation_full_closes_wholly_subfloor_debt_above_bad_debt_cleanup_threshold() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 20.0);
    t.borrow(ALICE, "ETH", 0.006);

    // ETH debt -> $8.10, below the $10 debt floor. USDC collateral -> $6.40,
    // above the $5 standalone bad-debt cleanup threshold but below debt.
    t.set_price("ETH", usd(1350));
    t.set_price("USDC", usd_cents(32));
    t.assert_liquidatable(ALICE);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.006);
    assert!(
        result.is_ok(),
        "full-covered wholly-sub-floor debt must close instead of becoming stuck: {result:?}"
    );
    assert!(
        t.borrow_balance(ALICE, "ETH") < 0.0001,
        "sub-floor ETH debt should be fully closed"
    );
}

// The same wholly-sub-floor account must also be clearable by the normal
// profitable path: repay up to available collateral, seize it, then socialize
// the residual bad debt inline. Otherwise only an overpaying keeper could clear
// this debt band.
#[test]
fn test_liquidation_partial_socializes_wholly_subfloor_debt_after_seizing_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 20.0);
    t.borrow(ALICE, "ETH", 0.006);

    t.set_price("ETH", usd(1350));
    t.set_price("USDC", usd_cents(32));
    t.assert_liquidatable(ALICE);

    t.get_or_create_user(LIQUIDATOR);
    let liquidator_usdc_before = t.token_balance(LIQUIDATOR, "USDC");
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.0045);
    assert!(
        result.is_ok(),
        "profitable partial liquidation should seize collateral then socialize residual debt: {result:?}"
    );

    assert_eq!(t.borrow_balance(ALICE, "ETH"), 0.0);
    assert_eq!(t.supply_balance(ALICE, "USDC"), 0.0);
    assert!(
        t.token_balance(LIQUIDATOR, "USDC") > liquidator_usdc_before,
        "liquidator should receive seized collateral"
    );
}

// Regression for Codex adversarial-review #2: governance must be able
// to push a custom `max_utilization_ray` through `update_params` without
// the pool silently resetting it to 95 %.
#[test]
fn test_update_params_threads_custom_max_utilization() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");

    // Apply a non-default cap (85 %) via the controller's pool-params
    // update path.
    let model = controller::types::InterestRateModel {
        max_borrow_rate_ray: 2 * controller::constants::RAY,
        base_borrow_rate_ray: controller::constants::RAY / 100,
        slope1_ray: controller::constants::RAY * 4 / 100,
        slope2_ray: controller::constants::RAY * 10 / 100,
        slope3_ray: controller::constants::RAY * 80 / 100,
        mid_utilization_ray: controller::constants::RAY * 50 / 100,
        optimal_utilization_ray: controller::constants::RAY * 80 / 100,
        // Non-default — must survive the round-trip.
        max_utilization_ray: controller::constants::RAY * 85 / 100,
        reserve_factor_bps: 1000,
    };
    t.ctrl_client()
        .upgrade_liquidity_pool_params(&asset, &model);

    // Read the pool's stored params through the harness view. The cap
    // must equal what we sent, not the previous default.
    let pool = t.pool_client("USDC");
    let sync = pool.get_sync_data(&asset);
    assert_eq!(
        sync.params.max_utilization_ray,
        controller::constants::RAY * 85 / 100,
        "update_params dropped max_utilization_ray"
    );
}

// Regression for Codex adversarial-review #5: disabled isolated markets
// must remain repayable so isolated borrowers can always close their
// debt and exit risk-reducing positions.
#[test]
fn test_isolated_repay_works_against_disabled_market() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market_config("USDC", |c| {
            c.is_isolated_asset = true;
            c.isolation_borrow_enabled = true;
            c.isolation_debt_ceiling_usd_wad = 1_000_000 * controller::constants::WAD;
        })
        .with_market(eth_preset())
        .with_market_config("ETH", |c| {
            c.isolation_borrow_enabled = true;
        })
        .build();

    // Alice opens an isolated USDC-backed account and borrows a small
    // ETH amount.
    t.create_isolated_account(ALICE, "USDC");
    t.supply(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "ETH", 0.05); // $100 debt

    // Operator disables the USDC market (e.g. deprecation).
    t.env.as_contract(&t.controller, || {
        let key = controller::types::ControllerKey::Market(t.resolve_asset("USDC"));
        let mut market: controller::types::MarketConfig =
            t.env.storage().persistent().get(&key).unwrap();
        market.status = controller::types::MarketStatus::Disabled;
        t.env.storage().persistent().set(&key, &market);
    });

    // Repay must still succeed — IsolatedRepay policy allows the
    // disabled-market reservation in the oracle gate.
    t.repay(ALICE, "ETH", 0.05);
}
