use controller::types::PositionMode;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Vec};
use test_harness::{
    assert_contract_error, errors, f64_to_i128, hub_asset, map_try_ok_value, FlashPositionMode,
    FlashPositionRequest, LendingTest, ALICE, BOB, HARNESS_SPOKE,
};

use crate::helpers::{collaterals, data, usdc_raw, AliceOps};

fn request(
    t: &LendingTest,
    mode: FlashPositionMode,
    collateral_amount: f64,
) -> FlashPositionRequest {
    FlashPositionRequest {
        mode,
        collateral: t.resolve_asset("USDC"),
        collateral_amount: usdc_raw(t, collateral_amount),
        extra_asset: Address::generate(&t.env),
        extra_amount: 0,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    }
}

fn setup() -> LendingTest {
    LendingTest::new().standard_two_asset().build()
}

#[test]
fn test_flash_position_opens_healthy_account_without_fee() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let refunds = Vec::new(&t.env);

    let revenue_before = t.snapshot_revenue("ETH");
    let account_id = t.flash_position(
        ALICE,
        0,
        PositionMode::Multiply,
        "ETH",
        1.0,
        &receiver,
        &payload,
        &mins,
        &refunds,
    );

    assert!(account_id > 0);
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (3_999.0..=4_001.0).contains(&supply),
        "USDC supply got {supply}"
    );
    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&borrow),
        "ETH borrow should be the full 1.0 with zero fee, got {borrow}"
    );
    let hf = t.health_factor_for(ALICE, account_id);
    assert!(hf >= 1.0, "HF got {hf}");
    assert_eq!(t.snapshot_revenue("ETH"), revenue_before);
}

#[test]
fn test_flash_position_extends_existing_account() {
    let mut t = setup();
    let account_id = t.create_account_full(ALICE, HARNESS_SPOKE, PositionMode::Multiply);
    t.supply(ALICE, "USDC", 10_000.0);

    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let id = t
        .try_flash_position(
            ALICE,
            account_id,
            PositionMode::Multiply,
            "ETH",
            1.0,
            &receiver,
            &payload,
            &mins,
            &Vec::new(&t.env),
        )
        .expect("existing account");
    assert_eq!(id, account_id);
    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!((0.99..=1.01).contains(&borrow));
}

#[test]
fn test_flash_position_same_asset_loop() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 200.0));
    let mins = collaterals(&t, &[("USDC", 200.0)]);
    let account_id = t.flash_position(
        ALICE,
        0,
        PositionMode::Multiply,
        "USDC",
        100.0,
        &receiver,
        &payload,
        &mins,
        &Vec::new(&t.env),
    );
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    let borrow = t.borrow_balance_for(ALICE, account_id, "USDC");
    assert!((199.0..=201.0).contains(&supply));
    assert!((99.0..=101.0).contains(&borrow));
    assert!(t.health_factor_for(ALICE, account_id) >= 1.0);
}

#[test]
fn test_flash_position_refuses_a_market_with_flash_loans_disabled() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market_config("ETH", |c| c.is_flashloanable = false)
        .build();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::FLASHLOAN_NOT_ENABLED);
}

#[test]
fn test_flash_position_rejects_empty_collaterals() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::KeepFunds, 0.0));
    let result = t.try_alice_eth_flash(&receiver, &payload, &Vec::new(&t.env), &Vec::new(&t.env));
    assert_contract_error(result, errors::INVALID_PAYMENTS);
}

#[test]
fn test_flash_position_rejects_all_zero_mins() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 0.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::COLLATERAL_REQUIRED);
}

#[test]
fn test_flash_position_rejects_below_minimum() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::BelowMin, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::COLLATERAL_MINIMUM_NOT_MET);
}

#[test]
fn test_flash_position_rejects_keep_funds() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::KeepFunds, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::COLLATERAL_MINIMUM_NOT_MET);
}

#[test]
fn test_flash_position_rejects_non_contract_receiver() {
    let mut t = setup();
    let eoa = Address::generate(&t.env);
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&eoa, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::INVALID_FLASHLOAN_RECEIVER);
}

#[test]
fn test_flash_position_rejects_controller_receiver() {
    let mut t = setup();
    let controller = t.controller.clone();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&controller, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::INVALID_FLASHLOAN_RECEIVER);
}

#[test]
fn test_flash_position_reenter_supply_rejects() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::ReenterSupply, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    // Nested controller entry from the callback must fail. The host may surface
    // InvalidAction for same-contract reentry; FLASH_LOAN_ONGOING is pinned by
    // `test_flash_position_rejects_during_flash_loan`.
    assert!(
        result.is_err(),
        "callback must not reenter controller supply: {:?}",
        result
    );
    t.env.as_contract(&t.controller, || {
        assert!(
            !controller::test_support::is_flash_loan_ongoing(&t.env),
            "flash guard must clear on rollback"
        );
    });
}

#[test]
fn test_flash_position_rejects_when_paused() {
    let mut t = setup();
    t.pause();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::CONTRACT_PAUSED);
}

#[test]
fn test_flash_position_rejects_during_flash_loan() {
    let mut t = setup();
    t.set_flash_loan_ongoing(true);
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}

#[test]
fn test_flash_position_refunds_undeclared_push() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let extra = t.resolve_asset("ETH");
    let extra_amount = f64_to_i128(0.5, t.resolve_market("ETH").decimals);
    let req = FlashPositionRequest {
        mode: FlashPositionMode::Undeclared,
        collateral: t.resolve_asset("USDC"),
        collateral_amount: usdc_raw(&t, 4_000.0),
        extra_asset: extra.clone(),
        extra_amount,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    };
    let mut refunds = Vec::new(&t.env);
    refunds.push_back(extra.clone());
    let caller = t.get_or_create_user(ALICE);
    let eth_before = soroban_sdk::token::Client::new(&t.env, &extra).balance(&caller);

    let account_id = t
        .try_alice_eth_flash(
            &receiver,
            &data(&t, req),
            &collaterals(&t, &[("USDC", 4_000.0)]),
            &refunds,
        )
        .expect("undeclared refund");
    assert!(account_id > 0);
    let eth_after = soroban_sdk::token::Client::new(&t.env, &extra).balance(&caller);
    assert_eq!(eth_after - eth_before, extra_amount);
    assert_eq!(t.supply_balance_for(ALICE, account_id, "ETH"), 0.0);
}

#[test]
fn test_flash_position_pushing_debt_back_does_not_repay() {
    let mut t = setup();
    let account_id = t.create_account_full(ALICE, HARNESS_SPOKE, PositionMode::Multiply);
    t.supply(ALICE, "USDC", 20_000.0);
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::PushDebtBack, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 1.0)]);
    let mut refunds = Vec::new(&t.env);
    refunds.push_back(t.resolve_asset("ETH"));

    let result = t.try_flash_position(
        ALICE,
        account_id,
        PositionMode::Multiply,
        "ETH",
        1.0,
        &receiver,
        &payload,
        &mins,
        &refunds,
    );
    // No USDC is pushed (PushDebtBack only returns the debt token), so the
    // declared 1.0 USDC minimum is not met. This is the round-trip denial.
    assert_contract_error(result, errors::COLLATERAL_MINIMUM_NOT_MET);
}

#[test]
fn test_flash_position_rejects_paused_collateral_before_callback() {
    let mut t = setup();
    t.set_spoke_asset_paused("USDC", true);
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::SPOKE_ASSET_PAUSED);
}

#[test]
fn test_flash_position_rejects_non_owner() {
    let mut t = setup();
    t.create_account_full(ALICE, HARNESS_SPOKE, PositionMode::Multiply);
    let alice_id = t.resolve_account_id(ALICE);
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_flash_position(
        BOB,
        alice_id,
        PositionMode::Multiply,
        "ETH",
        1.0,
        &receiver,
        &payload,
        &mins,
        &Vec::new(&t.env),
    );
    assert_contract_error(result, errors::NOT_AUTHORIZED);
}

#[test]
fn test_flash_position_returning_debt_token_does_not_repay() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let eth = t.resolve_asset("ETH");
    let payload = data(
        &t,
        request(&t, FlashPositionMode::SupplyAndReturnDebt, 4_000.0),
    );
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let mut refunds = Vec::new(&t.env);
    refunds.push_back(eth.clone());
    let caller = t.get_or_create_user(ALICE);
    let eth_before = soroban_sdk::token::Client::new(&t.env, &eth).balance(&caller);
    let revenue_before = t.snapshot_revenue("ETH");
    let minted = f64_to_i128(1.0, t.resolve_market("ETH").decimals);

    let account_id = t
        .try_alice_eth_flash(&receiver, &payload, &mins, &refunds)
        .expect("mins met plus returned debt must still leave the position open");

    let debt = t
        .ctrl_client()
        .get_borrow_amount(&account_id, &hub_asset(eth.clone()));
    assert_eq!(debt, minted, "returning the debt token must not repay");
    assert_eq!(t.snapshot_revenue("ETH"), revenue_before);
    let eth_after = soroban_sdk::token::Client::new(&t.env, &eth).balance(&caller);
    assert_eq!(eth_after - eth_before, minted);
}

#[test]
fn test_flash_position_dust_collateral_fails_solvency() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 0.0000001));
    let mut mins = Vec::new(&t.env);
    mins.push_back((hub_asset(t.resolve_asset("USDC")), 1i128));
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}

#[test]
fn test_flash_position_rejects_duplicate_collateral() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let usdc = hub_asset(t.resolve_asset("USDC"));
    let min = usdc_raw(&t, 4_000.0);
    let mut mins = Vec::new(&t.env);
    mins.push_back((usdc.clone(), min));
    mins.push_back((usdc, min));
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::INVALID_PAYMENTS);
}

#[test]
fn test_flash_position_rejects_refund_overlap() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let mut refunds = Vec::new(&t.env);
    refunds.push_back(t.resolve_asset("USDC"));
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &refunds);
    assert_contract_error(result, errors::INVALID_PAYMENTS);
}

#[test]
fn test_flash_position_rejects_pool_receiver() {
    let mut t = setup();
    let pool = t.markets.get("ETH").expect("ETH market").pool.clone();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&pool, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::INVALID_FLASHLOAN_RECEIVER);
}

#[test]
fn test_flash_position_rejects_mode_mismatch() {
    let mut t = setup();
    t.create_account_full(ALICE, HARNESS_SPOKE, PositionMode::Multiply);
    let alice_id = t.resolve_account_id(ALICE);
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_flash_position(
        ALICE,
        alice_id,
        PositionMode::Long,
        "ETH",
        1.0,
        &receiver,
        &payload,
        &mins,
        &Vec::new(&t.env),
    );
    assert_contract_error(result, errors::ACCOUNT_MODE_MISMATCH);
}

#[test]
fn test_flash_position_rejects_frozen_collateral() {
    let mut t = setup();
    t.set_spoke_asset_flags("USDC", false, true, false);
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::SPOKE_ASSET_FROZEN);
}

#[test]
fn test_flash_position_rejects_zero_amount() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_flash_position(
        ALICE,
        0,
        PositionMode::Multiply,
        "ETH",
        0.0,
        &receiver,
        &payload,
        &mins,
        &Vec::new(&t.env),
    );
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

#[test]
fn test_flash_position_rejects_paused_debt() {
    let mut t = setup();
    t.set_spoke_asset_paused("ETH", true);
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::SPOKE_ASSET_PAUSED);
}

#[test]
fn test_flash_position_rejects_non_borrowable_debt() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market_config("ETH", |c| c.is_borrowable = false)
        .build();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::ASSET_NOT_BORROWABLE);
}

#[test]
fn test_flash_position_rejects_non_collateralizable() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market_config("USDC", |c| c.is_collateralizable = false)
        .build();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::NOT_COLLATERAL);
}

#[test]
fn test_flash_position_rejects_spoke_mismatch() {
    let mut t = setup();
    t.create_account_full(ALICE, HARNESS_SPOKE, PositionMode::Multiply);
    let alice_id = t.resolve_account_id(ALICE);
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let caller = t.get_or_create_user(ALICE);
    let debt = hub_asset(t.resolve_asset("ETH"));
    let result = map_try_ok_value(t.ctrl_client().try_flash_position(
        &caller,
        &alice_id,
        &99u32,
        &PositionMode::Multiply,
        &debt,
        &f64_to_i128(1.0, 7),
        &receiver,
        &payload,
        &mins,
        &Vec::new(&t.env),
    ));
    assert_contract_error(result, errors::SPOKE_MISMATCH);
}

#[test]
fn test_flash_position_rejects_duplicate_refund_assets() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let eth = t.resolve_asset("ETH");
    let mut refunds = Vec::new(&t.env);
    refunds.push_back(eth.clone());
    refunds.push_back(eth);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &refunds);
    assert_contract_error(result, errors::INVALID_PAYMENTS);
}

#[test]
fn test_flash_position_callback_panic_rolls_back() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Panic, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let revenue_before = t.snapshot_revenue("ETH");
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert!(result.is_err(), "panic callback must revert: {result:?}");
    t.env.as_contract(&t.controller, || {
        assert!(!controller::test_support::is_flash_loan_ongoing(&t.env));
    });
    assert_eq!(t.snapshot_revenue("ETH"), revenue_before);
}

#[test]
fn test_flash_position_mixed_zero_and_positive_min_succeeds() {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mut mins = Vec::new(&t.env);
    mins.push_back((hub_asset(t.resolve_asset("ETH")), 0i128));
    mins.push_back((hub_asset(t.resolve_asset("USDC")), usdc_raw(&t, 4_000.0)));
    let account_id = t
        .try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env))
        .expect("zero min on unused asset plus positive USDC min");
    let minted = f64_to_i128(1.0, t.resolve_market("ETH").decimals);
    let debt = t
        .ctrl_client()
        .get_borrow_amount(&account_id, &hub_asset(t.resolve_asset("ETH")));
    assert_eq!(debt, minted);
    assert!(t.supply_balance_for(ALICE, account_id, "USDC") > 0.0);
}

#[test]
fn test_flash_position_keep_funds_on_existing_healthy_account_leaves_debt() {
    let mut t = setup();
    let account_id = t.create_account_full(ALICE, HARNESS_SPOKE, PositionMode::Multiply);
    t.supply(ALICE, "USDC", 20_000.0);
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 1.0));
    let mins = collaterals(&t, &[("USDC", 1.0)]);
    let minted = f64_to_i128(1.0, t.resolve_market("ETH").decimals);
    let id = t
        .try_flash_position(
            ALICE,
            account_id,
            PositionMode::Multiply,
            "ETH",
            1.0,
            &receiver,
            &payload,
            &mins,
            &Vec::new(&t.env),
        )
        .expect("existing spare HF plus dust collateral");
    assert_eq!(id, account_id);
    let debt = t
        .ctrl_client()
        .get_borrow_amount(&account_id, &hub_asset(t.resolve_asset("ETH")));
    assert_eq!(debt, minted);
    assert!(
        t.health_factor_for(ALICE, account_id) >= 1.0,
        "must not leave bad debt"
    );
}
