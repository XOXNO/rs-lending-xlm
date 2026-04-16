extern crate std;

use common::types::ControllerKey;
use test_harness::{
    days, eth_preset, usd_cents, usdc_preset, LendingTest, ALICE, BOB, STABLECOIN_EMODE,
};

fn supply_threshold_bps(t: &LendingTest, account_id: u64, asset_name: &str) -> i128 {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller_address(), || {
        t.env
            .storage()
            .persistent()
            .get::<_, common::types::AccountPosition>(&ControllerKey::SupplyPosition(
                account_id, asset,
            ))
            .expect("supply position should exist")
            .liquidation_threshold_bps
    })
}

// ---------------------------------------------------------------------------
// 1. test_update_indexes_refreshes_rates
// ---------------------------------------------------------------------------

#[test]
fn test_update_indexes_refreshes_rates() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Setup: supply + borrow to create utilization
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    let borrow_before = t.borrow_balance(ALICE, "ETH");

    // Advance time and sync indexes
    t.advance_and_sync(days(30));

    let borrow_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        borrow_after > borrow_before,
        "borrow balance should increase after index update: before={}, after={}",
        borrow_before,
        borrow_after
    );
}

// ---------------------------------------------------------------------------
// 2. test_clean_bad_debt_removes_positions
// ---------------------------------------------------------------------------

#[test]
fn test_clean_bad_debt_removes_positions() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies small USDC and borrows ETH near limit
    t.supply(ALICE, "USDC", 10.0); // $10 collateral
    t.borrow(ALICE, "ETH", 0.003); // ~$6 debt

    // Crash USDC price so collateral becomes nearly worthless and below $5
    // $10 * $0.01 = $0.10 collateral (< $5 bad debt threshold)
    t.set_price("USDC", usd_cents(1));

    // Verify account can be liquidated
    assert!(t.can_be_liquidated(ALICE), "Alice should be liquidatable");

    // Clean bad debt
    t.clean_bad_debt_for(ALICE);

    // After cleaning bad debt, positions should be removed
    t.assert_no_positions(ALICE);
}

// ---------------------------------------------------------------------------
// 3. test_clean_bad_debt_rejects_healthy
// ---------------------------------------------------------------------------

#[test]
fn test_clean_bad_debt_rejects_healthy() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice with healthy position
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.assert_healthy(ALICE);

    let account_id = t.resolve_account_id(ALICE);
    let result = t.try_clean_bad_debt_by_id(account_id);
    assert!(
        result.is_err(),
        "clean_bad_debt should fail on healthy account"
    );
}

// ---------------------------------------------------------------------------
// 4. test_clean_bad_debt_rejects_above_threshold
// ---------------------------------------------------------------------------

#[test]
fn test_clean_bad_debt_rejects_above_threshold() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice supplies significant collateral and borrows near limit
    t.supply(ALICE, "USDC", 1000.0); // $1000 collateral
    t.borrow(ALICE, "ETH", 0.3); // ~$600 debt

    // Drop USDC price to make liquidatable but collateral > $5
    // $1000 * $0.50 = $500 collateral (well above $5 threshold)
    t.set_price("USDC", usd_cents(50));

    // Should be liquidatable
    assert!(t.can_be_liquidated(ALICE), "Alice should be liquidatable");

    // But collateral is above $5 threshold, so clean_bad_debt should fail
    let account_id = t.resolve_account_id(ALICE);
    let result = t.try_clean_bad_debt_by_id(account_id);
    assert!(
        result.is_err(),
        "clean_bad_debt should fail when collateral > $5"
    );
}

// ---------------------------------------------------------------------------
// 5. test_update_account_threshold_safe
// ---------------------------------------------------------------------------

#[test]
fn test_update_account_threshold_safe() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let hf_before = t.health_factor(ALICE);
    let account_id = t.resolve_account_id(ALICE);

    // Update safe params (has_risks=false): LTV, bonus, fees
    // This should succeed without HF check
    t.update_account_threshold("USDC", false, &[account_id]);

    // Position should still exist and be healthy
    t.assert_healthy(ALICE);

    // Verify the account's health factor is still valid after threshold propagation
    let hf_after = t.health_factor(ALICE);
    assert!(
        hf_after >= 1.0,
        "HF should remain healthy after safe threshold update: before={}, after={}",
        hf_before,
        hf_after
    );
}

// ---------------------------------------------------------------------------
// 6. test_update_account_threshold_risky
// ---------------------------------------------------------------------------

#[test]
fn test_update_account_threshold_risky() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0); // ~$2000 debt on $100k collateral -> very healthy

    let hf_before = t.health_factor(ALICE);
    let account_id = t.resolve_account_id(ALICE);

    // Update risky params (has_risks=true): liquidation threshold
    // This should trigger HF check but pass since HF is very high
    t.update_account_threshold("USDC", true, &[account_id]);

    t.assert_healthy(ALICE);

    // Verify the HF is still valid after risky threshold update
    let hf_after = t.health_factor(ALICE);
    assert!(
        hf_after >= 1.0,
        "HF should remain healthy after risky threshold update: before={}, after={}",
        hf_before,
        hf_after
    );
}

// ---------------------------------------------------------------------------
// 7. test_update_account_threshold_rejects_low_hf
// ---------------------------------------------------------------------------

#[test]
fn test_update_account_threshold_rejects_low_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply and borrow near limit so HF is close to 1.0
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0); // ~$6000 debt on $10k collateral, HF ~ 1.33

    let account_id = t.resolve_account_id(ALICE);

    // Lower the threshold in the config so HF would drop below 1.05 safety buffer.
    // Must also lower LTV to remain below threshold (contract validates threshold > LTV).
    // $10k * 61% = $6100 weighted collateral / $6000 debt = HF ~1.017 < 1.05
    t.edit_asset_config("USDC", |c| {
        c.loan_to_value_bps = 5000;
        c.liquidation_threshold_bps = 6100;
    });

    let result = t.try_update_account_threshold("USDC", true, &[account_id]);
    assert!(
        result.is_err(),
        "update_account_threshold should fail when HF < 1.05 after update"
    );
}

// ---------------------------------------------------------------------------
// 8. test_update_account_threshold_deprecated_emode_uses_base_params
// ---------------------------------------------------------------------------

#[test]
fn test_update_account_threshold_deprecated_emode_uses_base_params() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .build();

    let account_id = t.create_emode_account(ALICE, 1);
    t.supply_to(ALICE, account_id, "USDC", 1_000.0);

    assert_eq!(supply_threshold_bps(&t, account_id, "USDC"), 9800);

    t.remove_e_mode_category(1);
    t.update_account_threshold("USDC", true, &[account_id]);

    assert_eq!(
        supply_threshold_bps(&t, account_id, "USDC"),
        8000,
        "deprecated eMode categories should fall back to base asset thresholds during propagation"
    );
}

// ---------------------------------------------------------------------------
// 9. test_keeper_role_required
// ---------------------------------------------------------------------------

#[test]
fn test_keeper_role_required() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Create BOB without KEEPER role
    let bob_addr = t.get_or_create_user(BOB);

    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::vec![&t.env, t.resolve_market("USDC").asset.clone()];

    // BOB tries `update_indexes` without the KEEPER role.
    // Use bare `is_err()` because Soroban wraps cross-contract errors at the
    // outer caller boundary.
    let result = ctrl.try_update_indexes(&bob_addr, &assets);
    assert!(
        result.is_err(),
        "non-keeper should not be able to call update_indexes"
    );

    // BOB tries clean_bad_debt without KEEPER role
    let result = ctrl.try_clean_bad_debt(&bob_addr, &999u64);
    assert!(
        result.is_err(),
        "non-keeper should not be able to call clean_bad_debt"
    );
}
