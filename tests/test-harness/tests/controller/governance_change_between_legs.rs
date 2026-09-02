//! GH-24, GH-25. Parameter changes that land between two user legs: a lower
//! LTV gates the next withdraw but leaves the stamped threshold alone; a
//! swapped router serves the next strategy; a lowered position limit blocks
//! the next new slot; a pause closes entries and keeps exits.

use controller::types::PositionMode;
use governance_interface::AdminOperation;
use soroban_sdk::token;
use test_harness::mock_aggregator::MockAggregator;
use test_harness::{
    assert_contract_error, build_aggregator_swap, errors, hub_asset, LendingTest, ALICE, BOB,
    HARNESS_SPOKE,
};

fn setup() -> LendingTest {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "WBTC", 10.0);
    t.fund_router("ETH", 100.0);
    t.fund_router("USDC", 100_000.0);
    t
}

#[test]
fn lowering_ltv_between_borrow_and_withdraw_gates_the_withdraw_but_not_the_stamp() {
    let mut t = setup();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.5);
    let account = t.account_id(ALICE);
    t.edit_asset_in_spoke("USDC", HARNESS_SPOKE, true, true, 6_000, 8_000, 500);
    assert_contract_error(
        t.try_withdraw(ALICE, "USDC", 1_000.0),
        errors::INSUFFICIENT_COLLATERAL,
    );
    // The rejected withdraw reverted its restamp; a top-up carries it.
    t.supply(ALICE, "USDC", 1.0);
    let (supplies, _) = t.ctrl_client().get_account_positions(&account);
    let stamp = supplies.get(hub_asset(t.resolve_asset("USDC"))).unwrap();
    assert_eq!(stamp.loan_to_value, 6_000, "LTV restamps on the action");
    assert_eq!(stamp.liquidation_threshold, 8_000, "the threshold does not");
    t.repay(ALICE, "ETH", 2.0);
    t.try_withdraw(ALICE, "USDC", 1_000.0)
        .expect("repay first, then the withdraw clears the new LTV");
}

#[test]
fn swapping_the_router_between_two_strategies_serves_the_second_from_the_new_router() {
    let mut t = setup();
    let alice = t.get_or_create_user(ALICE);
    let usdc_addr = t.resolve_asset("USDC");
    let usdc = hub_asset(usdc_addr.clone());
    let eth = hub_asset(t.resolve_asset("ETH"));
    let u = 10_000_000i128;
    t.resolve_market("USDC")
        .token_admin
        .mint(&alice, &(2_000 * u));
    // 0.5 ETH of strategy debt swaps into 1000 USDC on top of 2000 USDC paid in.
    let swap = build_aggregator_swap(&t, "ETH", "USDC", 0, 1_000 * u);
    let account = t.ctrl_client().multiply(
        &alice,
        &0,
        &HARNESS_SPOKE,
        &usdc,
        &(u / 2),
        &eth,
        &PositionMode::Multiply,
        &swap,
        &Some((usdc.clone(), 2_000 * u)),
        &None,
    );
    let old_router = t.aggregator.clone();
    let new_router = t.env.register(MockAggregator, (t.admin(),));
    t.gov_client().execute_immediate(
        &t.admin(),
        &AdminOperation::SetSwapAggregator(new_router.clone()),
    );
    t.resolve_market("USDC")
        .token_admin
        .mint(&new_router, &(100_000 * u));
    let usdc_token = token::Client::new(&t.env, &usdc_addr);
    let old_before = usdc_token.balance(&old_router);
    let new_before = usdc_token.balance(&new_router);
    t.ctrl_client().multiply(
        &alice,
        &account,
        &HARNESS_SPOKE,
        &usdc,
        &(u / 2),
        &eth,
        &PositionMode::Multiply,
        &swap,
        &None,
        &None,
    );
    assert!(
        t.borrow_balance_raw_for(account, "ETH") >= u,
        "both legs booked"
    );
    assert_eq!(
        usdc_token.balance(&old_router),
        old_before,
        "the old router served nothing"
    );
    assert_eq!(
        new_before - usdc_token.balance(&new_router),
        1_000 * u,
        "the second leg was paid by the new router"
    );
}

#[test]
fn lowering_the_position_limit_between_two_opens_blocks_only_the_new_slot() {
    let mut t = setup();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 0.5);
    t.set_position_limits(2, 1);
    assert_contract_error(
        t.try_borrow(ALICE, "WBTC", 0.01),
        errors::POSITION_LIMIT_EXCEEDED,
    );
    t.try_borrow(ALICE, "ETH", 0.1)
        .expect("a held debt asset is a top-up");
}

#[test]
fn a_pause_between_two_legs_closes_entries_and_keeps_exits() {
    let mut t = setup();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.pause();
    assert_contract_error(t.try_supply(ALICE, "USDC", 1.0), errors::CONTRACT_PAUSED);
    assert_contract_error(t.try_borrow(ALICE, "ETH", 0.1), errors::CONTRACT_PAUSED);
    t.try_repay(ALICE, "ETH", 0.5).expect("repay stays open");
    t.try_withdraw(ALICE, "USDC", 100.0)
        .expect("withdraw stays open");
}
