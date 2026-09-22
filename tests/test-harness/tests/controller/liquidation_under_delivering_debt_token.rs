//! A fee-on-transfer debt token shrinks both the debt retired and the collateral awarded.

use common::types::{AccountPositionRaw, SeizeMode};
use soroban_sdk::token;
use test_harness::{
    asset_payment_vec, eth_preset, hub_asset, usd_cents, usdc_preset, wbtc_preset, LendingTest,
    ALICE, LIQUIDATOR,
};

const SHORTFALL_BPS: i128 = 1_000; // the pool receives 90 % of what the liquidator sends
const SENT: i128 = 10_000_000; // 1 ETH, 7 decimals
const RECEIVED: i128 = SENT - SENT * SHORTFALL_BPS / 10_000;

/// Alice: 10 000 USDC collateral at $0.70, 3 ETH debt at $2 000. HF = 0.933, solvent.
fn liquidatable_with_fee_on_transfer_debt() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_fee_on_transfer_market(eth_preset(), SHORTFALL_BPS)
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(70));
    t.assert_liquidatable(ALICE);
    assert!(t.total_collateral_raw(ALICE) > t.total_debt_raw(ALICE));
    t
}

fn supply_position(t: &LendingTest, account_id: u64, asset_name: &str) -> AccountPositionRaw {
    t.ctrl_client()
        .get_account_positions(&account_id)
        .0
        .get(hub_asset(t.resolve_asset(asset_name)))
        .expect("supply position")
}

#[test]
fn transfer_seizure_is_scaled_to_the_debt_tokens_the_pool_received() {
    let mut t = liquidatable_with_fee_on_transfer_debt();
    let account_id = t.resolve_account_id(ALICE);
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    let usdc = token::Client::new(&t.env, &t.resolve_asset("USDC"));
    let eth = token::Client::new(&t.env, &t.resolve_asset("ETH"));
    let pool = t.resolve_market("ETH").pool.clone();

    let payments = asset_payment_vec(&t.env, t.resolve_asset("ETH"), SENT);
    let plan =
        t.ctrl_client()
            .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Transfer);
    let planned_gross = plan.seized_collaterals.get(0).unwrap().amount;
    let planned_fee = plan.protocol_fees.get(0).unwrap().amount;
    let bonus_bps = plan.bonus_rate_bps;
    assert!(plan.refunds.is_empty(), "1 ETH must be fully usable");

    let debt_before = t.borrow_balance_raw(ALICE, "ETH");
    let collateral_before = t.supply_balance_raw(ALICE, "USDC");
    let pool_eth_before = eth.balance(&pool);
    let liq_usdc_before = usdc.balance(&liquidator);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let pool_got = eth.balance(&pool) - pool_eth_before;
    let debt_drop = debt_before - t.borrow_balance_raw(ALICE, "ETH");
    let collateral_drop = collateral_before - t.supply_balance_raw(ALICE, "USDC");
    let liquidator_got = usdc.balance(&liquidator) - liq_usdc_before;

    // Independent expectation: $2 000 * (1 + bonus) of USDC at $0.70, fee = 12 % of the bonus.
    let indep_gross = 2_000 * (10_000 + bonus_bps) * 10_000_000 / 7_000;
    let indep_fee = (indep_gross - indep_gross * 10_000 / (10_000 + bonus_bps)) * 1_200 / 10_000;
    let indep_net_scaled = (indep_gross - indep_fee) * RECEIVED / SENT;

    let expected_gross = planned_gross * RECEIVED / SENT;
    let expected_net = expected_gross - planned_fee * RECEIVED / SENT;
    std::println!(
        "bonus_bps={bonus_bps} planned_gross={planned_gross} planned_fee={planned_fee} \
         pool_got={pool_got} debt_drop={debt_drop} collateral_drop={collateral_drop} \
         liquidator_got={liquidator_got} expected_net={expected_net} indep_net={indep_net_scaled}"
    );

    assert_eq!(pool_got, RECEIVED, "the token must under-deliver by 10 %");
    assert!(
        (debt_drop - RECEIVED).abs() <= 1,
        "debt must fall by the RECEIVED {RECEIVED}, not the sent {SENT}: fell {debt_drop}"
    );
    assert!(
        (collateral_drop - expected_gross).abs() <= 1,
        "collateral seized must be planned*received/sent = {expected_gross}, got {collateral_drop}"
    );
    assert!(
        (liquidator_got - expected_net).abs() <= 1,
        "liquidator must get the scaled net seizure {expected_net}, got {liquidator_got}"
    );
    assert!(
        (liquidator_got - indep_net_scaled).abs() <= 10,
        "scaled seizure must match the hand-computed value {indep_net_scaled}, got {liquidator_got}"
    );
    assert!(
        liquidator_got < planned_gross - planned_fee,
        "a partial payment must not earn the full planned seizure"
    );
}

#[test]
fn credit_seizure_is_scaled_to_the_debt_tokens_the_pool_received() {
    let mut t = liquidatable_with_fee_on_transfer_debt();
    let account_id = t.resolve_account_id(ALICE);

    let payments = asset_payment_vec(&t.env, t.resolve_asset("ETH"), SENT);
    let plan =
        t.ctrl_client()
            .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Credit(0));
    let planned_shares = plan.seized_collaterals.get(0).unwrap().amount;
    let planned_fee_shares = plan.protocol_fees.get(0).unwrap().amount;

    let debt_before = t.borrow_balance_raw(ALICE, "ETH");
    let alice_shares_before = supply_position(&t, account_id, "USDC").scaled_amount;

    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));

    let debt_drop = debt_before - t.borrow_balance_raw(ALICE, "ETH");
    let alice_share_drop =
        alice_shares_before - supply_position(&t, account_id, "USDC").scaled_amount;
    let credited = supply_position(&t, receiver, "USDC").scaled_amount;

    let expected_debit = planned_shares * RECEIVED / SENT;
    let expected_credit = (planned_shares - planned_fee_shares) * RECEIVED / SENT;
    std::println!(
        "planned_shares={planned_shares} planned_fee_shares={planned_fee_shares} \
         debt_drop={debt_drop} alice_share_drop={alice_share_drop} credited={credited} \
         expected_debit={expected_debit} expected_credit={expected_credit}"
    );

    assert!(
        (debt_drop - RECEIVED).abs() <= 1,
        "debt must fall by the RECEIVED {RECEIVED}, not the sent {SENT}: fell {debt_drop}"
    );
    assert!(
        (alice_share_drop - expected_debit).abs() <= 1,
        "shares debited must be planned*received/sent = {expected_debit}, got {alice_share_drop}"
    );
    // The credit fee is re-derived from the floored bonus base, so allow that one extra floor.
    assert!(
        (credited - expected_credit).abs() <= 2,
        "shares credited must be the scaled net seizure {expected_credit}, got {credited}"
    );
    assert!(
        credited < planned_shares - planned_fee_shares,
        "a partial payment must not earn the full planned share credit"
    );
    assert!(credited <= alice_share_drop, "credit cannot exceed debit");
}

#[test]
fn seizure_follows_the_aggregate_received_value_when_one_of_two_debt_legs_under_delivers() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_fee_on_transfer_market(eth_preset(), SHORTFALL_BPS)
        .with_market(wbtc_preset())
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.5); // $3 000
    t.borrow(ALICE, "WBTC", 0.05); // $3 000
    t.set_price("USDC", usd_cents(70));
    t.assert_liquidatable(ALICE);

    let account_id = t.resolve_account_id(ALICE);
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    let usdc = token::Client::new(&t.env, &t.resolve_asset("USDC"));
    // 0.5 ETH = $1 000 planned / $900 received; 0.01 WBTC = $600 planned and received.
    let (eth_sent, wbtc_sent) = (5_000_000i128, 100_000i128);
    t.resolve_market("ETH")
        .token_admin
        .mint(&liquidator, &eth_sent);
    t.resolve_market("WBTC")
        .token_admin
        .mint(&liquidator, &wbtc_sent);
    let payments = soroban_sdk::vec![
        &t.env,
        (hub_asset(t.resolve_asset("ETH")), eth_sent),
        (hub_asset(t.resolve_asset("WBTC")), wbtc_sent),
    ];

    let plan =
        t.ctrl_client()
            .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Transfer);
    assert!(plan.refunds.is_empty());
    let planned_gross = plan.seized_collaterals.get(0).unwrap().amount;
    let planned_fee = plan.protocol_fees.get(0).unwrap().amount;
    let (eth_debt, wbtc_debt) = (
        t.borrow_balance_raw(ALICE, "ETH"),
        t.borrow_balance_raw(ALICE, "WBTC"),
    );

    t.ctrl_client()
        .liquidate(&liquidator, &account_id, &payments, &SeizeMode::Transfer);

    let liquidator_got = usdc.balance(&liquidator);
    let expected_net = planned_gross * 1_500 / 1_600 - planned_fee * 1_500 / 1_600;
    std::println!(
        "planned_gross={planned_gross} planned_fee={planned_fee} liquidator_got={liquidator_got} \
         expected_net={expected_net}"
    );
    assert!((eth_debt - t.borrow_balance_raw(ALICE, "ETH") - eth_sent * 9 / 10).abs() <= 1);
    assert!((wbtc_debt - t.borrow_balance_raw(ALICE, "WBTC") - wbtc_sent).abs() <= 1);
    assert!(
        (liquidator_got - expected_net).abs() <= 1,
        "liquidator must get planned * 1500/1600 = {expected_net}, got {liquidator_got}"
    );
}
