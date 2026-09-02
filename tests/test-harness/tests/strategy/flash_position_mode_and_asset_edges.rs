//! GH-18, GH-19, GH-20. `flash_position` accepts only the three strategy
//! modes, refuses a market whose flash loans are disabled, and collapses to a
//! plain borrow when the debt asset is declared as collateral.

use common::types::HubAssetKey;
use controller::types::PositionMode;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{vec, Address, Bytes, Vec};
use test_harness::{
    assert_contract_error, build_aggregator_swap, errors, f64_to_i128, hub_asset,
    FlashPositionMode, FlashPositionRequest, LendingTest, ALICE, BOB, HARNESS_SPOKE,
};

fn request(t: &LendingTest, asset: &str, amount: f64) -> Bytes {
    FlashPositionRequest {
        mode: FlashPositionMode::Success,
        collateral: t.resolve_asset(asset),
        collateral_amount: f64_to_i128(amount, 7),
        extra_asset: Address::generate(&t.env),
        extra_amount: 0,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    }
    .to_xdr(&t.env)
}

fn mins(t: &LendingTest, asset: &str, amount: f64) -> Vec<(HubAssetKey, i128)> {
    vec![
        &t.env,
        (hub_asset(t.resolve_asset(asset)), f64_to_i128(amount, 7)),
    ]
}

#[test]
fn a_normal_mode_flash_position_is_rejected() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    let receiver = t.deploy_flash_position_receiver();
    let result = t.try_flash_position(
        ALICE,
        0,
        PositionMode::Normal,
        "ETH",
        1.0,
        &receiver,
        &request(&t, "USDC", 4_000.0),
        &mins(&t, "USDC", 4_000.0),
        &Vec::new(&t.env),
    );
    assert_contract_error(result.map(|_| ()), errors::INVALID_POSITION_MODE);
}

#[test]
fn the_three_strategy_modes_are_all_accepted() {
    for mode in [
        PositionMode::Multiply,
        PositionMode::Long,
        PositionMode::Short,
    ] {
        let mut t = LendingTest::new().standard_two_asset().build();
        t.supply(BOB, "ETH", 100.0);
        let receiver = t.deploy_flash_position_receiver();
        t.try_flash_position(
            ALICE,
            0,
            mode,
            "ETH",
            1.0,
            &receiver,
            &request(&t, "USDC", 4_000.0),
            &mins(&t, "USDC", 4_000.0),
            &Vec::new(&t.env),
        )
        .expect("strategy modes open");
    }
}

#[test]
fn a_market_with_flash_loans_disabled_refuses_flash_position_but_not_multiply() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market_config("ETH", |c| c.is_flashloanable = false)
        .build();
    t.supply(BOB, "ETH", 100.0);
    t.fund_router("USDC", 100_000.0);
    let receiver = t.deploy_flash_position_receiver();
    let result = t.try_flash_position(
        ALICE,
        0,
        PositionMode::Multiply,
        "ETH",
        1.0,
        &receiver,
        &request(&t, "USDC", 4_000.0),
        &mins(&t, "USDC", 4_000.0),
        &Vec::new(&t.env),
    );
    assert_contract_error(result.map(|_| ()), errors::FLASHLOAN_NOT_ENABLED);
    // 1 ETH of strategy debt swaps into 2000 USDC on top of 1000 USDC paid in.
    let alice = t.get_or_create_user(ALICE);
    t.resolve_market("USDC")
        .token_admin
        .mint(&alice, &f64_to_i128(1_000.0, 7));
    let swap = build_aggregator_swap(&t, "ETH", "USDC", 0, f64_to_i128(2_000.0, 7));
    let usdc = hub_asset(t.resolve_asset("USDC"));
    let id = t.ctrl_client().multiply(
        &alice,
        &0,
        &HARNESS_SPOKE,
        &usdc,
        &f64_to_i128(1.0, 7),
        &hub_asset(t.resolve_asset("ETH")),
        &PositionMode::Multiply,
        &swap,
        &Some((usdc.clone(), f64_to_i128(1_000.0, 7))),
        &None,
    );
    assert_eq!(
        t.borrow_balance_raw_for(id, "ETH"),
        f64_to_i128(1.0, 7),
        "multiply routes through the governance-owned router and stays open"
    );
}

#[test]
fn declaring_the_debt_asset_as_the_only_collateral_fails_the_solvency_gate() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    let receiver = t.deploy_flash_position_receiver();
    // The receiver pushes back exactly the debt it received: supply 1 ETH against 1 ETH of debt.
    let result = t.try_flash_position(
        ALICE,
        0,
        PositionMode::Multiply,
        "ETH",
        1.0,
        &receiver,
        &request(&t, "ETH", 1.0),
        &mins(&t, "ETH", 1.0),
        &Vec::new(&t.env),
    );
    assert_contract_error(result.map(|_| ()), errors::INSUFFICIENT_COLLATERAL);
}

#[test]
fn declaring_the_debt_asset_alongside_real_collateral_is_a_plain_leveraged_open() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    let receiver = t.deploy_flash_position_receiver();
    let payload = FlashPositionRequest {
        mode: FlashPositionMode::SupplyAndReturnDebt,
        collateral: t.resolve_asset("USDC"),
        collateral_amount: f64_to_i128(4_000.0, 7),
        extra_asset: Address::generate(&t.env),
        extra_amount: 0,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    }
    .to_xdr(&t.env);
    let both: Vec<(HubAssetKey, i128)> = vec![
        &t.env,
        (hub_asset(t.resolve_asset("USDC")), f64_to_i128(4_000.0, 7)),
        (hub_asset(t.resolve_asset("ETH")), 0),
    ];
    let id = t
        .try_flash_position(
            ALICE,
            0,
            PositionMode::Multiply,
            "ETH",
            1.0,
            &receiver,
            &payload,
            &both,
            &Vec::new(&t.env),
        )
        .expect("the returned debt token books as collateral; the debt stays");
    assert!(t.supply_balance_raw_for(id, "ETH") > 0);
    assert!(t.borrow_balance_raw_for(id, "ETH") > 0);
}
