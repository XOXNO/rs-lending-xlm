//! GH-16. Lowering the position limit below an account's current count must
//! not strand the account: topping up a held asset opens no slot and stays
//! allowed; opening a new slot is what the limit rejects.

use common::types::SeizeMode;
use test_harness::{
    assert_contract_error, errors, usd, LendingTest, ALICE, BOB, CAROL, LIQUIDATOR,
};

fn two_positions_then_limit_of_one() -> LendingTest {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(BOB, "WBTC", 10.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 2.0);
    t.set_position_limits(1, 1);
    t
}

#[test]
fn a_held_asset_can_still_be_topped_up_after_the_limit_drops_below_the_count() {
    let mut t = two_positions_then_limit_of_one();
    t.try_supply(ALICE, "USDC", 100.0)
        .expect("top-up opens no slot");
    t.try_supply_to_account(CAROL, ALICE, "USDC", 1.0)
        .expect("third-party top-up opens no slot");
}

#[test]
fn a_new_slot_is_still_rejected_after_the_limit_drops() {
    let mut t = two_positions_then_limit_of_one();
    assert_contract_error(
        t.try_supply(ALICE, "WBTC", 1.0),
        errors::POSITION_LIMIT_EXCEEDED,
    );
}

#[test]
fn a_credit_liquidation_into_an_over_limit_account_that_holds_the_asset_still_lands() {
    let mut t = two_positions_then_limit_of_one();
    // LIQUIDATOR holds USDC and ETH; the seized asset is USDC, which it already holds.
    t.set_position_limits(2, 2);
    t.supply(BOB, "ETH", 100.0);
    t.supply(LIQUIDATOR, "USDC", 1_000.0);
    t.supply(LIQUIDATOR, "ETH", 0.5);
    t.supply(CAROL, "USDC", 10_000.0);
    t.borrow(CAROL, "ETH", 3.0);
    t.set_price("ETH", usd(3_000));
    t.set_position_limits(1, 1);
    let receiver = t.account_id(LIQUIDATOR);
    t.try_liquidate_with_mode(LIQUIDATOR, CAROL, "ETH", 0.5, SeizeMode::Credit(receiver))
        .expect("seizing an asset the receiver already holds opens no slot");
}
