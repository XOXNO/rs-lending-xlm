//! The owner-only `force_socialize_bad_debt` has no dust cap, so its single
//! admission test `total_debt > total_collateral` (`liquidation/mod.rs:228`) is
//! the only thing between a liquidatable-but-solvent user and a collateral burn.

use crate::shared::get_indexes;
use controller::constants::WAD;
use test_harness::{assert_contract_error, errors, usd_cents, LendingTest, ALICE, BOB};

/// Alice: 10 000 USDC collateral, 3 ETH ($6 000) debt; Bob backs the ETH market.
fn alice_with_usdc_at(cents: i128) -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(cents));
    t
}

fn assert_refused_and_untouched(t: &LendingTest) {
    let account_id = t.resolve_account_id(ALICE);
    let (si_before, _) = get_indexes(t, "ETH");
    let supply_before = t.supply_balance_raw(ALICE, "USDC");
    let debt_before = t.borrow_balance_raw(ALICE, "ETH");

    let refused = t.try_force_socialize_bad_debt_by_id(account_id);
    assert_contract_error(refused, errors::CANNOT_CLEAN_BAD_DEBT);

    assert_eq!(get_indexes(t, "ETH").0, si_before);
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), supply_before);
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), debt_before);
}

#[test]
fn force_socialize_refuses_a_liquidatable_but_solvent_account() {
    let t = alice_with_usdc_at(70);
    t.assert_liquidatable(ALICE);
    let (collateral, debt) = (t.total_collateral_raw(ALICE), t.total_debt_raw(ALICE));
    assert_eq!((collateral, debt), (7_000 * WAD, 6_000 * WAD));
    assert_refused_and_untouched(&t);
}

#[test]
fn force_socialize_refuses_when_debt_equals_collateral() {
    let t = alice_with_usdc_at(60);
    t.assert_liquidatable(ALICE);
    let (collateral, debt) = (t.total_collateral_raw(ALICE), t.total_debt_raw(ALICE));
    assert_eq!((collateral, debt), (6_000 * WAD, 6_000 * WAD));
    assert_refused_and_untouched(&t);
}

#[test]
fn force_socialize_admits_one_price_step_below_equality() {
    let mut t = alice_with_usdc_at(60);
    // 1e4 raw WAD is the mock feed's smallest step (14-decimal prices).
    t.set_price("USDC", usd_cents(60) - 10_000);
    let (collateral, debt) = (t.total_collateral_raw(ALICE), t.total_debt_raw(ALICE));
    assert!(debt > collateral, "debt={debt} collateral={collateral}");
    assert!(debt - collateral < WAD / 1_000_000, "gap must be minimal");

    let (si_before, _) = get_indexes(&t, "ETH");
    t.force_socialize_bad_debt_by_id(t.resolve_account_id(ALICE));
    assert!(get_indexes(&t, "ETH").0 < si_before);
    t.assert_no_positions(ALICE);
}
