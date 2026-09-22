use crate::shared::data_for_topic;
use common::types::SeizeMode;
use controller::constants::WAD;
use soroban_sdk::testutils::Events;
use soroban_sdk::vec;
use soroban_sdk::xdr::ScVal;
use test_harness::{
    assert_contract_error, eth_preset, hub_asset, map_try_ok_value, seed_band_usdc_eth,
    usdc_preset, LendingTest, ALICE, LIQUIDATOR,
};

const SAC_INSUFFICIENT_BALANCE: u32 = 10;

fn band_book() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset().build();
    seed_band_usdc_eth(&mut t);
    t
}

#[test]
fn a_band_full_close_with_duplicate_debt_legs_pulls_the_merged_offer() {
    let mut t = band_book();
    let account_id = t.resolve_account_id(ALICE);
    let eth = hub_asset(t.resolve_asset("ETH"));
    let debt = t.borrow_balance_raw(ALICE, "ETH");
    let payments = vec![&t.env, (eth.clone(), 2_0000000i128), (eth, 2_0000000i128)];
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market("ETH")
        .token_admin
        .mint(&liquidator, &4_0000000);
    let before = t.token_balance_raw(LIQUIDATOR, "ETH");

    t.ctrl_client()
        .liquidate(&liquidator, &account_id, &payments, &SeizeMode::Transfer);

    assert_eq!(
        t.borrow_balance_raw(ALICE, "ETH"),
        0,
        "the merged 4 ETH offer closes the debt"
    );
    assert_eq!(
        before - t.token_balance_raw(LIQUIDATOR, "ETH"),
        debt,
        "the pool refunds the merged offer above the debt"
    );
}

#[test]
fn a_fee_on_transfer_band_full_close_retires_the_debt_and_refunds_the_received_excess() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_fee_on_transfer_market(eth_preset(), 1_000)
        .build();
    seed_band_usdc_eth(&mut t);
    let account_id = t.resolve_account_id(ALICE);
    let debt = t.borrow_balance_raw(ALICE, "ETH");
    let offer = 4_0000000i128;
    let hub = hub_asset(t.resolve_asset("ETH"));
    let payments = vec![&t.env, (hub.clone(), offer)];
    let estimate =
        t.ctrl_client()
            .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Transfer);
    let seized = estimate.seized_collaterals.get(0).expect("USDC leg").amount;
    let fee = estimate.protocol_fees.get(0).expect("USDC fee").amount;
    let cash_before = t.pool_client("ETH").get_reserves(&hub);
    t.get_or_create_user(LIQUIDATOR);
    let usdc_before = t.token_balance_raw(LIQUIDATOR, "USDC");

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 4.0);

    let received = offer - offer * 1_000 / 10_000;
    let refund_sent = received - debt;
    assert_eq!(
        t.borrow_balance_raw(ALICE, "ETH"),
        0,
        "3.6 ETH received covers the debt"
    );
    assert_eq!(
        t.pool_client("ETH").get_reserves(&hub) - cash_before,
        debt,
        "pool cash grows by exactly the retired debt"
    );
    assert_eq!(
        t.token_balance_raw(LIQUIDATOR, "ETH"),
        refund_sent - refund_sent * 1_000 / 10_000,
        "the pool refunds what it received above the debt"
    );
    assert_eq!(
        t.token_balance_raw(LIQUIDATOR, "USDC") - usdc_before,
        seized - fee,
        "a full receipt leaves the seizure unscaled"
    );
}

#[test]
fn a_band_over_offer_above_the_liquidator_balance_reverts_with_the_token_balance_error() {
    let mut t = band_book();
    let account_id = t.resolve_account_id(ALICE);
    let debt = t.borrow_balance_raw(ALICE, "ETH");
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market("ETH").token_admin.mint(&liquidator, &debt);
    let payments = vec![&t.env, (hub_asset(t.resolve_asset("ETH")), 2 * debt)];

    let result = map_try_ok_value(t.ctrl_client().try_liquidate(
        &liquidator,
        &account_id,
        &payments,
        &SeizeMode::Transfer,
    ));
    assert_contract_error(result, SAC_INSUFFICIENT_BALANCE);
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), debt);
    assert_eq!(t.token_balance_raw(LIQUIDATOR, "ETH"), debt);
}

#[test]
fn a_band_over_offer_liquidation_event_reports_the_credited_debt_not_the_pull() {
    let mut t = band_book();
    let account_id = t.resolve_account_id(ALICE);
    let payments = vec![&t.env, (hub_asset(t.resolve_asset("ETH")), 4_0000000i128)];
    let estimate =
        t.ctrl_client()
            .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Transfer);
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market("ETH")
        .token_admin
        .mint(&liquidator, &4_0000000);

    t.ctrl_client()
        .liquidate(&liquidator, &account_id, &payments, &SeizeMode::Transfer);

    let events = t.env.events().all();
    let liquidations = data_for_topic(&events, "position", "liquidation");
    assert_eq!(liquidations.len(), 1);
    let ScVal::Map(Some(map)) = &liquidations[0] else {
        panic!("liquidation event data is a map");
    };
    let field = |name: &str| -> ScVal {
        map.iter()
            .find(|e| matches!(&e.key, ScVal::Symbol(s) if s.0.to_string() == name))
            .unwrap_or_else(|| panic!("no field `{name}`"))
            .val
            .clone()
    };
    let (ScVal::I128(repaid), ScVal::I128(bonus)) = (field("repaid_usd_wad"), field("bonus_bps"))
    else {
        panic!("repaid_usd_wad and bonus_bps are i128");
    };
    assert_eq!(
        estimate.max_payment_wad,
        6_000 * WAD,
        "the 3 ETH debt is worth $6 000"
    );
    assert_eq!(
        i128::from(&repaid),
        estimate.max_payment_wad,
        "the event reports the 3 ETH credited, not the 4 ETH pulled"
    );
    assert_eq!(i128::from(&bonus), 333);
}

#[test]
fn an_exact_cover_full_close_leaves_the_floored_collateral_unit_with_the_debt_free_account() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "ETH", 0.3375);
    let debt_tokens = t.borrow_balance_raw(ALICE, "ETH");
    t.set_price(
        "ETH",
        t.total_collateral_raw(ALICE) * 10_000_000 / debt_tokens,
    );
    let (collateral, debt) = (t.total_collateral_raw(ALICE), t.total_debt_raw(ALICE));
    assert_eq!(collateral - debt, 1_000, "C sits a few WAD units above D");
    let account_id = t.resolve_account_id(ALICE);
    let offer = debt_tokens * 3 / 2;
    let payments = vec![&t.env, (hub_asset(t.resolve_asset("ETH")), offer)];
    let estimate =
        t.ctrl_client()
            .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Transfer);
    assert_eq!(
        estimate.bonus_rate_bps, 0,
        "the negative cap is clamped to zero"
    );
    let usdc_held = t.supply_balance_raw(ALICE, "USDC");
    let revenue_before = t.snapshot_revenue("USDC");
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market("ETH")
        .token_admin
        .mint(&liquidator, &offer);
    let usdc_before = t.token_balance_raw(LIQUIDATOR, "USDC");

    t.ctrl_client()
        .liquidate(&liquidator, &account_id, &payments, &SeizeMode::Transfer);

    assert_eq!(t.total_debt_raw(ALICE), 0, "no debt is left to socialize");
    assert_eq!(
        t.supply_balance_raw(ALICE, "USDC"),
        1,
        "seizure flooring leaves one USDC unit as collateral"
    );
    assert_eq!(
        t.token_balance_raw(LIQUIDATOR, "USDC") - usdc_before,
        usdc_held - 1,
        "the liquidator receives C less the floored unit"
    );
    assert_eq!(
        t.snapshot_revenue("USDC"),
        revenue_before,
        "a debt-free account is not swept by bad-debt cleanup"
    );
}
