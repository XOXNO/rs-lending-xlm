use controller::types::ControllerKey;
use test_harness::{hub_asset, HubAssetKey,
    assert_contract_error, days, errors, eth_preset, usd_cents, usdc_preset, LendingTest, ALICE,
    BOB, STABLECOIN_EMODE,
};

fn supply_threshold_bps(t: &LendingTest, account_id: u64, asset_name: &str) -> u32 {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller_address(), || {
        let map: soroban_sdk::Map<HubAssetKey, controller::types::AccountPositionRaw> = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::SupplyPositions(account_id))
            .expect("supply side map should exist");
        map.get(hub_asset(asset))
            .expect("supply position should exist for asset")
            .liquidation_threshold_bps
    })
}

/// Returns stored supply risk fields as `(threshold, bonus, ltv)` BPS.
fn supply_risk_fields(t: &LendingTest, account_id: u64, asset_name: &str) -> (u32, u32, u32) {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller_address(), || {
        let map: soroban_sdk::Map<HubAssetKey, controller::types::AccountPositionRaw> = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::SupplyPositions(account_id))
            .expect("supply side map should exist");
        let p = map
            .get(hub_asset(asset))
            .expect("supply position should exist for asset");
        (
            p.liquidation_threshold_bps,
            p.liquidation_bonus_bps,
            p.loan_to_value_bps,
        )
    })
}

// Invariant guard for the borrow/collateral type split: the pool's position
// return must merge ONLY the scaled amount back onto a supply position — it
// must never zero the collateral risk fields the controller holds. A
// regression here makes HF math see 0% LTV everywhere and blocks all borrows.
#[test]
fn test_supply_roundtrip_preserves_risk_fields() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 1_000.0);
    let id = t.resolve_account_id(ALICE);
    let first = supply_risk_fields(&t, id, "USDC");
    assert!(
        first.0 > 0 && first.2 > 0,
        "preset should seed non-zero threshold/ltv; got {:?}",
        first
    );

    // Second supply round-trips through the pool and merges the returned
    // position back onto the stored one.
    t.supply(ALICE, "USDC", 1_000.0);
    let second = supply_risk_fields(&t, id, "USDC");

    assert_eq!(
        first, second,
        "supply round-trip must preserve (threshold, bonus, ltv); pool return \
         merge zeroed risk fields"
    );
}
// 1. test_update_indexes_refreshes_rates

#[test]
fn test_update_indexes_refreshes_rates() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    // Supply + borrow to create utilization.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    let borrow_before = t.borrow_balance(ALICE, "ETH");

    // Advance time and sync indexes.
    t.advance_and_sync(days(30));

    let borrow_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        borrow_after > borrow_before,
        "borrow balance should increase after index update: before={}, after={}",
        borrow_before,
        borrow_after
    );
}
// 2. test_clean_bad_debt_removes_positions

#[test]
fn test_clean_bad_debt_removes_positions() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    // Alice supplies small USDC and borrows ETH near the limit.
    t.supply(ALICE, "USDC", 10.0); // $10 collateral
    t.borrow(ALICE, "ETH", 0.003); // ~$6 debt

    // Crash USDC price so collateral becomes nearly worthless and falls below $5.
    // $10 * $0.01 = $0.10 collateral (< $5 bad-debt threshold).
    t.set_price("USDC", usd_cents(1));

    // Verify the account can be liquidated.
    assert!(t.can_be_liquidated(ALICE), "Alice should be liquidatable");

    // Clean bad debt.
    t.clean_bad_debt_for(ALICE);

    // After cleaning bad debt, positions must be removed.
    t.assert_no_positions(ALICE);
}
// 3. test_clean_bad_debt_rejects_healthy

#[test]
fn test_clean_bad_debt_rejects_healthy() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    // Alice with a healthy position.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.assert_healthy(ALICE);

    let account_id = t.resolve_account_id(ALICE);
    let result = t.try_clean_bad_debt_by_id(account_id);
    assert_contract_error(result, errors::CANNOT_CLEAN_BAD_DEBT);
}
// 4. test_clean_bad_debt_rejects_above_threshold

#[test]
fn test_clean_bad_debt_rejects_above_threshold() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    // Alice supplies significant collateral and borrows near the limit.
    t.supply(ALICE, "USDC", 1000.0); // $1000 collateral
    t.borrow(ALICE, "ETH", 0.3); // ~$600 debt

    // Drop USDC price to make Alice liquidatable while collateral > $5.
    // $1000 * $0.50 = $500 collateral (well above the $5 threshold).
    t.set_price("USDC", usd_cents(50));

    // Should be liquidatable.
    assert!(t.can_be_liquidated(ALICE), "Alice should be liquidatable");

    // Collateral is above the $5 threshold, so clean_bad_debt must fail.
    let account_id = t.resolve_account_id(ALICE);
    let result = t.try_clean_bad_debt_by_id(account_id);
    assert_contract_error(result, errors::CANNOT_CLEAN_BAD_DEBT);
}
// 4a. test_bad_debt_gap_band_resolved_by_liquidation
//
// Finding #2 — the ($5, total_debt) gap band. An insolvent account whose
// collateral sits ABOVE the $5 socialization threshold but BELOW its debt is
// NOT directly socializable (`clean_bad_debt` reverts `CannotCleanBadDebt`),
// yet it is NOT protocol-stuck: a single liquidation's max-collateral close
// seizes the collateral down to <=$5, which triggers the IN-TX bad-debt
// socialization (`liquidation.rs` `will_socialize`) and winds the account down
// — without ever calling `clean_bad_debt`. So there IS a guaranteed protocol
// terminal transition; the only residual is whether a liquidator is
// economically motivated to act, which is off-chain (not assertable on-chain).
#[test]
fn test_bad_debt_gap_band_resolved_by_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    // $1000 USDC collateral, ~$600 ETH debt (within LTV).
    t.supply(ALICE, "USDC", 1000.0);
    t.borrow(ALICE, "ETH", 0.3);

    // Crash USDC to $0.30: collateral = $300. Now $5 < collateral ($300) <
    // debt ($600) with HF < 1 — squarely inside the gap band.
    t.set_price("USDC", usd_cents(30));
    assert!(t.can_be_liquidated(ALICE), "account should be insolvent");

    // (1) Direct socialization is gated out while collateral > $5.
    let account_id = t.resolve_account_id(ALICE);
    let gated = t.try_clean_bad_debt_by_id(account_id);
    assert_contract_error(gated, errors::CANNOT_CLEAN_BAD_DEBT);

    // (2) A liquidation resolves it anyway. Repaying the full debt lets the
    // contract cap the repaid amount to what the ~$300 collateral covers,
    // seize all of it (post-state <=$5), and socialize the residual in-tx.
    t.liquidate(BOB, ALICE, "ETH", 0.3);

    // (3) Terminal transition reached without clean_bad_debt: the account is
    // wound down and no longer insolvent.
    t.assert_no_positions(ALICE);
    assert!(
        !t.can_be_liquidated_by_id(account_id),
        "account must be resolved after the gap-band liquidation"
    );
}
// 4b. test_clean_bad_debt_rejected_under_oracle_deviation
//
// Standalone bad-debt cleanup reads the live price under the fail-closed
// oracle: when the primary and anchor sources diverge beyond the tolerance
// band, the read rejects with `UnsafePriceNotAllowed` instead of resolving to
// a price only one source corroborates. Cleanup is only permitted on prices
// both independent sources agree on within tolerance.
//
// The two oracles are independent, so sustained out-of-band divergence is
// implausible — transient gaps stay inside the tolerated band — making the
// rejection window narrow.
#[test]
fn test_clean_bad_debt_rejected_under_oracle_deviation() {
    use test_harness::TIGHT_TOLERANCE;

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    // Tight tolerance so a small primary/anchor gap counts as
    // deviation.
    t.set_oracle_tolerance("USDC", TIGHT_TOLERANCE);

    // Set up the bad-debt position: tiny collateral, much larger
    // debt. Mirror `test_clean_bad_debt_removes_positions`.
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    // Crash the aggregator price (live spot) so collateral falls
    // below the $5 threshold.
    t.set_price("USDC", usd_cents(1));

    // Skew the TWAP/anchor source so primary and anchor disagree beyond the
    // tolerance band; the fail-closed read rejects rather than resolving to a
    // singly-corroborated price.
    t.set_safe_price("USDC", usd_cents(100), false, false);

    // `clean_bad_debt` reads the live price: the out-of-band primary/anchor gap
    // is rejected with `UnsafePriceNotAllowed` rather than resolving to the
    // deviated aggregator price.
    let account_id = t.resolve_account_id(ALICE);
    let result = t.try_clean_bad_debt_by_id(account_id);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}
// 5. test_update_account_threshold_safe

#[test]
fn test_update_account_threshold_safe() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let hf_before = t.health_factor(ALICE);
    let account_id = t.resolve_account_id(ALICE);

    // Update safe params (has_risks=false): LTV, bonus, fees.
    // Should succeed without an HF check.
    t.update_account_threshold(false, &[account_id]);

    // Position should still exist and stay healthy.
    t.assert_healthy(ALICE);

    // Verify the account's health factor is still valid after threshold propagation.
    let hf_after = t.health_factor(ALICE);
    assert!(
        hf_after >= 1.0,
        "HF should remain healthy after safe threshold update: before={}, after={}",
        hf_before,
        hf_after
    );
}
// 6. test_update_account_threshold_risky

#[test]
fn test_update_account_threshold_risky() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0); // ~$2000 debt on $100k collateral -> very healthy

    let hf_before = t.health_factor(ALICE);
    let account_id = t.resolve_account_id(ALICE);

    // Update risky params (has_risks=true): liquidation threshold.
    // Should trigger the HF check but pass since HF is very high.
    t.update_account_threshold(true, &[account_id]);

    t.assert_healthy(ALICE);

    // Verify the HF is still valid after the risky threshold update.
    let hf_after = t.health_factor(ALICE);
    assert!(
        hf_after >= 1.0,
        "HF should remain healthy after risky threshold update: before={}, after={}",
        hf_before,
        hf_after
    );
}
// 7. test_update_account_threshold_rejects_low_hf

#[test]
fn test_update_account_threshold_rejects_low_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    // Supply and borrow near the limit so HF stays close to 1.0.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0); // ~$6000 debt on $10k collateral, HF ~ 1.33

    let account_id = t.resolve_account_id(ALICE);

    // Lower the threshold so HF drops below the 1.05 safety buffer.
    // Also lower LTV to remain below the threshold (the contract validates
    // threshold > LTV).
    // $10k * 61% = $6100 weighted collateral / $6000 debt = HF ~1.017 < 1.05.
    t.edit_asset_config("USDC", |c| {
        c.loan_to_value_bps = 5000;
        c.liquidation_threshold_bps = 6100;
    });

    let result = t.try_update_account_threshold(true, &[account_id]);
    assert_contract_error(result, errors::HEALTH_FACTOR_TOO_LOW);
}
// 8. test_update_account_threshold_deprecated_emode_retains_spoke_params

#[test]
fn test_update_account_threshold_deprecated_emode_retains_spoke_params() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_dust_disabled_all_markets()
        .build();

    let account_id = t.create_emode_account(ALICE, 1);
    t.supply_to(ALICE, account_id, "USDC", 1_000.0);

    assert_eq!(supply_threshold_bps(&t, account_id, "USDC"), 9800);

    // Spokes are self-contained: a deprecated spoke keeps its stored
    // `SpokeAsset` entry, so re-stamping a position on that spoke reads the
    // same spoke config -- there is no spoke-0 fallback (controller emode.rs).
    t.remove_e_mode_category(1);
    t.update_account_threshold(true, &[account_id]);

    assert_eq!(
        supply_threshold_bps(&t, account_id, "USDC"),
        9800,
        "a deprecated spoke's positions keep reading the spoke's own threshold (no spoke-0 fallback)"
    );
}

#[test]
fn test_update_account_threshold_syncs_all_supply_assets() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "ETH", 0.5);

    let account_id = t.resolve_account_id(ALICE);
    let (usdc_threshold_before, _, _) = supply_risk_fields(&t, account_id, "USDC");
    let (eth_threshold_before, _, _) = supply_risk_fields(&t, account_id, "ETH");
    assert_ne!(usdc_threshold_before, 6100);
    assert_ne!(eth_threshold_before, 6100);

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value_bps = 5000;
        c.liquidation_threshold_bps = 6100;
    });
    t.edit_asset_config("ETH", |c| {
        c.loan_to_value_bps = 5000;
        c.liquidation_threshold_bps = 6100;
    });

    t.update_account_threshold(true, &[account_id]);

    assert_eq!(supply_threshold_bps(&t, account_id, "USDC"), 6100);
    assert_eq!(supply_threshold_bps(&t, account_id, "ETH"), 6100);
    t.assert_healthy(ALICE);
}
// 9. test_permissionless_keeper_endpoints

#[test]
fn test_permissionless_keeper_endpoints() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    let bob_addr = t.get_or_create_user(BOB);

    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::vec![&t.env, hub_asset(t.resolve_market("USDC").asset.clone())];

    t.env.mock_all_auths();
    let result = ctrl.try_update_indexes(&bob_addr, &assets);
    assert!(result.is_ok(), "any signed caller may update_indexes");
}
