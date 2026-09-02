//! GH-23. `remove_spoke` has no usage check and deprecation is one-way, so a
//! spoke can hold live positions forever. Liquidation must stay possible for
//! a fresh liquidator there: `Credit(0)` creates the receiver account even in
//! a deprecated spoke, while every other account-creating verb stays closed.

use common::types::SeizeMode;
use controller::types::PositionMode;
use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usdc_preset, LendingTest, MarketPreset, ALICE,
    BOB, LIQUIDATOR, STABLECOIN_SPOKE,
};

const SPOKE: u32 = 2;

fn deprecated_spoke_with_an_unhealthy_account() -> (LendingTest, u64) {
    // Zero seed liquidity: the only USDC cash is what the victim supplies.
    let mut t = LendingTest::new()
        .with_market(MarketPreset {
            initial_liquidity: 0.0,
            ..usdc_preset()
        })
        .with_market(MarketPreset {
            initial_liquidity: 0.0,
            ..eth_preset()
        })
        .with_spoke(SPOKE, STABLECOIN_SPOKE)
        .with_spoke_asset(SPOKE, "USDC", true, true)
        .with_spoke_asset(SPOKE, "ETH", true, true)
        .with_max_utilization_disabled_all_markets()
        .build();
    let victim = t.create_spoke_account(ALICE, SPOKE);
    t.supply_to(ALICE, victim, "USDC", 10_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.borrow_to(ALICE, victim, "ETH", 4.5);
    t.remove_spoke_category(SPOKE);
    // Drain the USDC market so a Transfer-mode seizure has no cash to pay out.
    t.borrow(BOB, "USDC", 9_700.0);
    t.set_price("ETH", usd(2_500));
    assert!(t.can_be_liquidated_by_id(victim));
    (t, victim)
}

#[test]
fn transfer_mode_fails_on_cash_and_credit_zero_creates_the_receiver_in_the_deprecated_spoke() {
    let (mut t, victim) = deprecated_spoke_with_an_unhealthy_account();
    assert_contract_error(
        t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Transfer)
            .map(|_| ()),
        errors::INSUFFICIENT_LIQUIDITY,
    );
    let receiver = t
        .try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0))
        .expect("a seizure receiver may be created in a deprecated spoke");
    assert!(receiver > 0 && receiver != victim);
    assert_eq!(
        t.ctrl_client().get_account_attributes(&receiver).spoke_id,
        SPOKE
    );
}

#[test]
fn every_other_account_creating_verb_stays_closed_in_the_deprecated_spoke() {
    let (mut t, _) = deprecated_spoke_with_an_unhealthy_account();
    assert_contract_error(
        t.try_supply_with_spoke(LIQUIDATOR, "USDC", 1.0, SPOKE),
        errors::SPOKE_DEPRECATED,
    );
    let steps = t.mock_swap_steps("ETH", "USDC", 0);
    assert_contract_error(
        t.try_multiply_with_category(
            LIQUIDATOR,
            SPOKE,
            "USDC",
            0.1,
            "ETH",
            PositionMode::Multiply,
            &steps,
        )
        .map(|_| ()),
        errors::SPOKE_DEPRECATED,
    );
}
