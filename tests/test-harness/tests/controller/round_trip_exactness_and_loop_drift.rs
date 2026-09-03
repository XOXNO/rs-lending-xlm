//! GH-01. The directed-rounding policy end to end: what a user puts in is the
//! most they can take out, at every index, after any number of repetitions.

use common::math::fp::Ray;
use common::types::HubAssetKey;
use soroban_sdk::{vec, Vec};
use test_harness::{days, hub_asset, LendingTest, ALICE, BOB};

const USDC_UNIT: i128 = 10_000_000;
const ETH_UNIT: i128 = 10_000_000;

fn setup() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(BOB, "ETH", 100.0);
    t
}

/// Supplier claims never exceed cash plus outstanding debt.
fn pool_backing_holds(t: &LendingTest, asset: &str) {
    let key = hub_asset(t.resolve_asset(asset));
    let state = t.pool_client(asset).get_sync_data(&key).state;
    let supplied_value = Ray::from(state.supplied).mul_floor(&t.env, Ray::from(state.supply_index));
    let debt_value = Ray::from(state.borrowed).mul_ceil(&t.env, Ray::from(state.borrow_index));
    let cash_ray = Ray::from_asset(&t.env, state.cash, 7);
    assert!(
        supplied_value <= cash_ray.checked_add(&t.env, debt_value),
        "{asset}: supplier claims {} exceed cash {} plus debt {}",
        supplied_value.raw(),
        cash_ray.raw(),
        debt_value.raw()
    );
}

fn leg(t: &LendingTest, asset: &str, amount: i128) -> Vec<(HubAssetKey, i128)> {
    vec![&t.env, (hub_asset(t.resolve_asset(asset)), amount)]
}

#[test]
fn supply_then_withdraw_all_on_a_fresh_market_returns_exactly_the_deposit() {
    let mut t = setup();
    let deposit = 12_345 * USDC_UNIT + 6_789;
    t.supply_raw(ALICE, "USDC", deposit);
    assert_eq!(t.token_balance_raw(ALICE, "USDC"), 0);
    t.withdraw_all(ALICE, "USDC");
    assert_eq!(t.token_balance_raw(ALICE, "USDC"), deposit);
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), 0);
}

#[test]
fn withdraw_all_after_index_growth_never_exceeds_deposit_plus_accrued_interest() {
    let mut t = setup();
    let deposit = 10_000 * USDC_UNIT;
    t.supply_raw(BOB, "USDC", deposit);
    t.supply_raw(ALICE, "USDC", deposit);
    // BOB borrows against ETH so the USDC market accrues. Sized so ALICE's
    // full exit leaves utilization under the ceiling.
    t.borrow(BOB, "USDC", 4_000.0);
    t.advance_and_sync(days(365));
    let claimed_before = t.supply_balance_raw(ALICE, "USDC");
    t.withdraw_all(ALICE, "USDC");
    let paid = t.token_balance_raw(ALICE, "USDC");
    assert!(
        paid >= deposit,
        "interest cannot make the exit smaller than the deposit"
    );
    assert!(
        paid <= claimed_before,
        "the full close pays the floor value, never above the half-up view: paid {paid}, view {claimed_before}"
    );
    pool_backing_holds(&t, "USDC");
}

#[test]
fn two_hundred_supply_borrow_repay_withdraw_loops_extract_nothing() {
    let mut t = setup();
    let usdc_bankroll = 1_000_000 * USDC_UNIT;
    let eth_bankroll = ETH_UNIT;
    let alice = t.get_or_create_user(ALICE);
    t.resolve_market("USDC")
        .token_admin
        .mint(&alice, &usdc_bankroll);
    t.resolve_market("ETH")
        .token_admin
        .mint(&alice, &eth_bankroll);
    t.supply_raw(ALICE, "USDC", 10_000 * USDC_UNIT);
    let usdc_start = t.token_balance_raw(ALICE, "USDC") + t.supply_balance_raw(ALICE, "USDC");
    let account_id = t.account_id(ALICE);

    for i in 0..200u32 {
        t.ctrl_client().supply(
            &alice,
            &account_id,
            &1,
            &leg(&t, "USDC", 1_000_000 + i as i128),
        );
        t.ctrl_client().borrow(
            &alice,
            &account_id,
            &leg(&t, "ETH", 100_000 + i as i128),
            &None,
        );
        let debt = t.borrow_balance_raw(ALICE, "ETH") + 1;
        t.ctrl_client()
            .repay(&alice, &account_id, &leg(&t, "ETH", debt));
        assert_eq!(
            t.borrow_balance_raw(ALICE, "ETH"),
            0,
            "loop {i}: debt must clear"
        );
        t.ctrl_client().withdraw(
            &alice,
            &account_id,
            &leg(&t, "USDC", 1_000_000 + i as i128),
            &None,
        );
    }

    let usdc_end = t.token_balance_raw(ALICE, "USDC") + t.supply_balance_raw(ALICE, "USDC");
    assert!(
        usdc_end <= usdc_start,
        "USDC grew across loops: {usdc_start} -> {usdc_end}"
    );
    assert!(
        usdc_start - usdc_end <= 2 * 200,
        "USDC rounding loss above one unit per leg: {}",
        usdc_start - usdc_end
    );
    let eth_end = t.token_balance_raw(ALICE, "ETH");
    assert!(eth_end <= eth_bankroll, "ETH grew across loops");
    assert!(
        eth_bankroll - eth_end <= 2 * 200,
        "ETH rounding loss above one unit per repay: {}",
        eth_bankroll - eth_end
    );
    pool_backing_holds(&t, "USDC");
    pool_backing_holds(&t, "ETH");
}

#[test]
fn fifty_supply_withdraw_all_loops_never_return_more_than_deposited_plus_interest() {
    let mut t = setup();
    let alice = t.get_or_create_user(ALICE);
    let bankroll = 100_000 * USDC_UNIT;
    t.resolve_market("USDC").token_admin.mint(&alice, &bankroll);
    t.supply_raw(BOB, "USDC", 50_000 * USDC_UNIT);
    t.borrow(BOB, "USDC", 30_000.0);
    let per_loop = 777 * USDC_UNIT;
    for i in 0..50u32 {
        let deposit = per_loop + i as i128;
        // A full exit burns the account, so every loop opens a fresh one.
        let account_id = t
            .ctrl_client()
            .supply(&alice, &0, &1, &leg(&t, "USDC", deposit));
        t.advance_and_sync(days(3));
        t.ctrl_client()
            .withdraw(&alice, &account_id, &leg(&t, "USDC", 0), &None);
        assert!(
            !t.account_exists(account_id),
            "loop {i}: the emptied account burns"
        );
    }
    let wallet = t.token_balance_raw(ALICE, "USDC");
    assert!(
        wallet >= bankroll - 50,
        "rounding loss must stay below one unit per loop: {}",
        bankroll - wallet
    );
    // Three days at roughly 60 percent utilization earns well under one
    // percent of the deposit per loop.
    assert!(
        wallet <= bankroll + 50 * per_loop / 100,
        "interest above one percent per three-day loop: {}",
        wallet - bankroll
    );
    pool_backing_holds(&t, "USDC");
}
