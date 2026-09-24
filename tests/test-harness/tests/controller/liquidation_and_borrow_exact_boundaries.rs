//! Health factor exactly 1.0, a borrow exactly at the LTV limit, collateral exactly at the floor.

use common::types::HubAssetKey;
use controller::constants::WAD;
use soroban_sdk::{vec, Vec};
use test_harness::{
    assert_contract_error, errors, hub_asset, map_try_ok_unit, usd, LendingTest, ALICE, LIQUIDATOR,
};

fn try_borrow_raw(t: &LendingTest, asset: &str, raw: i128) -> Result<(), soroban_sdk::Error> {
    let caller = t.users.get(ALICE).unwrap().address.clone();
    let account_id = t.resolve_account_id(ALICE);
    let borrows: Vec<(HubAssetKey, i128)> = vec![&t.env, (hub_asset(t.resolve_asset(asset)), raw)];
    map_try_ok_unit(
        t.ctrl_client()
            .try_borrow(&caller, &account_id, &borrows, &None),
    )
}

#[test]
fn liquidate_is_rejected_at_health_factor_exactly_one_and_admitted_one_step_below() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.set_price("ETH", usd(1_000));
    t.borrow(ALICE, "ETH", 2.0);
    t.set_price("ETH", usd(4_000));
    assert_eq!(t.health_factor_raw(ALICE), WAD, "must land exactly on 1.0");

    let debt_before = t.borrow_balance_raw(ALICE, "ETH");
    let healthy = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert_contract_error(healthy, errors::HEALTH_FACTOR_TOO_HIGH);
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), debt_before);

    // 1e4 raw WAD is the mock feed's smallest step (14-decimal prices).
    t.set_price("ETH", usd(4_000) + 10_000);
    let hf = t.health_factor_raw(ALICE);
    std::println!("hf one step below = {hf}");
    // debt = 2 * ($4 000 + 1e-14) => HF = 1 / (1 + 2.5e-18), floored = WAD - 3.
    assert_eq!(
        hf,
        WAD - 3,
        "one price step is the closest reachable HF below 1.0"
    );

    t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5)
        .expect("HF one unit below 1.0 must be liquidatable");
    assert!(t.borrow_balance_raw(ALICE, "ETH") < debt_before);
}

#[test]
fn borrow_passes_exactly_at_the_ltv_limit_and_reverts_one_unit_above() {
    let mut t = LendingTest::new().standard_two_asset().build();
    // $10 000 at 75 % LTV = $7 500 = exactly 3.75 ETH = 37_500_000 units.
    t.supply(ALICE, "USDC", 10_000.0);
    let limit = 37_500_000i128;

    assert_contract_error(
        try_borrow_raw(&t, "ETH", limit + 1),
        errors::INSUFFICIENT_COLLATERAL,
    );
    assert_eq!(
        t.borrow_balance_raw(ALICE, "ETH"),
        0,
        "revert must persist nothing"
    );

    try_borrow_raw(&t, "ETH", limit).expect("borrow exactly at the LTV limit");
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), limit);
    let account_id = t.resolve_account_id(ALICE);
    assert_eq!(
        t.ctrl_client().get_ltv_collateral_usd(&account_id),
        t.total_debt_raw(ALICE),
        "LTV collateral must equal debt exactly"
    );

    assert_contract_error(
        try_borrow_raw(&t, "ETH", 1),
        errors::INSUFFICIENT_COLLATERAL,
    );
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), limit);
}

#[test]
fn borrow_passes_with_ltv_collateral_exactly_at_the_floor_and_reverts_one_unit_below() {
    // At 75 % LTV no USDC supply lands exactly on the default $5 floor; move the floor
    // onto the LTV collateral.
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 100.0); // LTV collateral = exactly $75
    let account_id = t.resolve_account_id(ALICE);
    let ltv_collateral = t.ctrl_client().get_ltv_collateral_usd(&account_id);
    assert_eq!(ltv_collateral, 75 * WAD);

    let admin = t.admin();
    let set_floor = |t: &LendingTest, floor: i128| {
        t.gov_client().execute_immediate(
            &admin,
            &governance::op::AdminOperation::SetMinBorrowCollateralUsd(floor),
        );
    };

    set_floor(&t, 75 * WAD + 1);
    assert_contract_error(
        try_borrow_raw(&t, "ETH", 10_000),
        errors::MIN_BORROW_COLLATERAL_NOT_MET,
    );
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), 0);

    set_floor(&t, 75 * WAD);
    try_borrow_raw(&t, "ETH", 10_000).expect("collateral exactly at the floor must borrow");
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), 10_000);
}
