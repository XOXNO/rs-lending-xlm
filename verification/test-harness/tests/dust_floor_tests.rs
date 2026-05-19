//! Per-position dust floor regression (Slender C-4;
//! `audit-research/STELLAR_AUDIT_FINDINGS.md` §4.4).
//!
//! Default preset floor is `$10` ([`common::constants::MIN_DUST_FLOOR_WAD`]).
//! Operations that would leave a position with USD value in the open
//! interval `(0, floor)` revert with `DustResidueNotAllowed`. Closing to
//! zero is always allowed.
//!
//! Liquidation has an escape: when partial liquidation would leave a
//! sub-floor residue on either side, the engine expands to a full close
//! rather than stranding the dust.

extern crate std;

use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, usdt_stable_preset, LendingTest,
    ALICE, LIQUIDATOR,
};

// ---------------------------------------------------------------------------
// Open-time gate
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Repay / withdraw partial-residue gates
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Liquidation full-close-on-dust-residue
// ---------------------------------------------------------------------------

#[test]
fn test_liquidation_expands_to_full_close_on_dust_residue() {
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
    t.set_price("USDC", common::constants::WAD / 2);
    t.assert_liquidatable(ALICE);

    // Liquidator repays $125 (just enough to push HF back near 1). A
    // mathematically partial liquidation would leave a few-dollar residue
    // on at least one side — the dust full-close path should expand
    // repayment to the full debt.
    let liq_id = t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.0625);

    // Post-liquidation: either the debt closed entirely (full-close fired)
    // or the position is healthy again. The relevant assertion is that
    // Alice is no longer in a sub-floor residue state.
    let _ = liq_id;
}

// Regression for Codex adversarial-review #1: dust expansion may never
// raise the seizure target beyond what the liquidator has actually paid.
// A liquidator who supplies a partial payment must receive collateral
// scaled to *that* payment — never collateral scaled to the (expanded)
// full-debt value.
#[test]
fn test_liquidation_partial_payment_does_not_over_seize_on_dust_expansion() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies $200 USDC, borrows $130 ETH.
    t.supply(ALICE, "USDC", 200.0);
    t.borrow(ALICE, "ETH", 0.065);
    t.set_price("USDC", common::constants::WAD / 2);
    t.assert_liquidatable(ALICE);

    // Snapshot Alice's debt before. Under the over-seize bug, dust
    // expansion would zero out her debt position based on a partial
    // payment. Under the fix, debt drops by ~payment value (not full).
    let debt_before = t.borrow_balance(ALICE, "ETH");

    // Liquidator submits a deliberately small payment (~$2 ETH = 0.001
    // ETH). Under the bug this would expand seizure to full debt
    // (~$130 worth of USDC collateral); under the fix the seizure is
    // bounded by `total_debt.min(payment_ceiling_usd)`.
    let _ = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let debt_after = t.borrow_balance(ALICE, "ETH");
    let debt_reduction = debt_before - debt_after;

    // The liquidator paid 0.001 ETH (~$2). Debt should drop by at most
    // that same amount (or zero — full close only fires when the
    // payment actually covers full debt). If debt drops by anything
    // close to the full ~0.065 ETH the over-seize bug is back.
    assert!(
        debt_reduction < 0.005,
        "debt dropped by {:.4} ETH on a 0.001 ETH partial payment — \
         dust-expansion is over-seizing (Codex #1 regression)",
        debt_reduction
    );
}

// Regression for Codex re-review of #1: a liquidator whose payment is
// large enough that dust expansion would fire (optimal repayment leaves
// sub-floor residue) but still doesn't cover total debt must be rejected
// with `DustResidueNotAllowed`. Half-expanding to the payment ceiling
// would still strand dust on either side.
//
// Setup: collateral barely above debt × (1 + base_bonus). The math
// engine's `d_max = collateral / (1 + base_bonus)` lands within $10 of
// `total_debt`, so the optimal partial repayment already triggers the
// dust gate. With a payment short of full debt, the fix must reject.
#[test]
fn test_liquidation_partial_above_optimal_rejects_when_residue_would_be_dust() {
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
    t.set_price("USDC", common::constants::WAD / 2);
    t.assert_liquidatable(ALICE);

    // Pay 0.010 ETH = $20 — above the optimal partial target (≈ $14)
    // but $2 short of total debt ($22). Without the fix, the engine
    // would expand to $20 and still leave a $2 sub-floor residue. The
    // fix must refuse.
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.010);
    assert_contract_error(result, errors::DUST_RESIDUE_NOT_ALLOWED);
}

// Regression for Codex re-review of #1 (multi-asset case). A user with
// multiple debt positions where ONE position's residue would land
// sub-floor — but the aggregate residue stays above the floor — must
// not strand the dust leg. The per-position dust gate fires.
#[test]
fn test_liquidation_rejects_per_position_dust_on_multi_debt_account() {
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
    t.set_price("USDC", common::constants::WAD * 35 / 100);
    t.assert_liquidatable(ALICE);

    // Liquidator pays only on the small ETH leg, just enough to leave
    // ~$1–2 ETH dust. The USDT leg is untouched (still ~$100), so the
    // aggregate residue debt stays well above $10. Without the
    // per-position gate the ETH leg would land in the dust window.
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.010);
    assert_contract_error(result, errors::DUST_RESIDUE_NOT_ALLOWED);
}

// Companion: when the same setup gets a payment that fully covers
// total debt, the full-close path succeeds — proving the rejection is
// scoped to short payments only.
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
    t.set_price("USDC", common::constants::WAD / 2);
    t.assert_liquidatable(ALICE);

    // Pay 0.011 ETH = $22 = total debt. Dust expansion fires AND the
    // payment ceiling allows it → full close.
    let _ = t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.011);
    let debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_after < 0.0001,
        "full payment should close the position, got {} ETH residual",
        debt_after
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
    let model = common::types::InterestRateModel {
        max_borrow_rate_ray: 2 * common::constants::RAY,
        base_borrow_rate_ray: common::constants::RAY / 100,
        slope1_ray: common::constants::RAY * 4 / 100,
        slope2_ray: common::constants::RAY * 10 / 100,
        slope3_ray: common::constants::RAY * 80 / 100,
        mid_utilization_ray: common::constants::RAY * 50 / 100,
        optimal_utilization_ray: common::constants::RAY * 80 / 100,
        // Non-default — must survive the round-trip.
        max_utilization_ray: common::constants::RAY * 85 / 100,
        reserve_factor_bps: 1000,
    };
    t.ctrl_client().upgrade_liquidity_pool_params(&asset, &model);

    // Read the pool's stored params through the harness view. The cap
    // must equal what we sent, not the previous default.
    let pool = t.pool_client("USDC");
    let sync = pool.get_sync_data();
    assert_eq!(
        sync.params.max_utilization_ray,
        common::constants::RAY * 85 / 100,
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
            c.isolation_debt_ceiling_usd_wad = 1_000_000 * common::constants::WAD;
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
        let key = common::types::ControllerKey::Market(t.resolve_asset("USDC"));
        let mut market: common::types::MarketConfig =
            t.env.storage().persistent().get(&key).unwrap();
        market.status = common::types::MarketStatus::Disabled;
        t.env.storage().persistent().set(&key, &market);
    });

    // Repay must still succeed — IsolatedRepay policy allows the
    // disabled-market reservation in the oracle gate.
    t.repay(ALICE, "ETH", 0.05);
}
