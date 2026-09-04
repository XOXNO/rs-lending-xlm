//! Spoke usage is a derived total: for every `(spoke, hub asset, side)` it must
//! equal the sum of the scaled positions it caps. Every share writer updates it
//! through the same leg merge, so the invariant holds after each step of a
//! flow that exercises all of them. The check itself lives in the harness
//! (`assert_spoke_usage_matches_positions`) so scenarios can call it anywhere.
//!
//! This is the positive pin for the A080 carve-out: `apply_exit` no-ops on a
//! missing row, so a row can only go missing if a writer skips `apply_leg_usage`.
//! No production writer does; this flow is where a future one would show up.

use common::types::SeizeMode;
use test_harness::{usd_cents, LendingTest, ALICE, BOB, CAROL, DAVE, LIQUIDATOR};

/// Two accounts in the harness spoke plus a third in the stablecoin spoke, so
/// the per-spoke split is exercised as well as the per-asset one. The dust
/// floor is off so a fourth, tiny account can end the flow inside the
/// bad-debt gate (collateral at or below `BAD_DEBT_USD_THRESHOLD`).
fn two_spokes() -> LendingTest {
    LendingTest::new()
        .with_market(test_harness::eth_preset())
        .stablecoin_spoke_two_asset()
        .with_dust_disabled_all_markets()
        .build()
}

#[test]
fn usage_equals_positions_across_supply_borrow_withdraw_repay_and_both_seize_modes() {
    let mut t = two_spokes();
    t.assert_spoke_usage_matches_positions();

    // Entry on both sides, in two spokes.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.supply(BOB, "ETH", 50.0);
    t.borrow(BOB, "USDC", 4_000.0);
    t.create_spoke_account(CAROL, 2);
    t.supply(CAROL, "USDC", 5_000.0);
    t.borrow(CAROL, "USDT", 1_000.0);
    t.supply(DAVE, "USDC", 10.0);
    t.borrow(DAVE, "ETH", 0.003);
    t.assert_spoke_usage_matches_positions();

    // Partial exits, then a full exit that prunes a position.
    t.withdraw(ALICE, "USDC", 500.0);
    t.repay(BOB, "USDC", 1_000.0);
    t.repay(CAROL, "USDT", 1_000.0);
    t.withdraw_all(CAROL, "USDC");
    t.assert_spoke_usage_matches_positions();

    // Interest accrues; scaled amounts do not move but the check must still
    // hold against the same scaled book.
    t.advance_time(test_harness::days(30));
    t.assert_spoke_usage_matches_positions();

    // Transfer seize burns shares out of the spoke; Credit(0) moves them to a
    // fresh account in the same spoke and exits only the protocol fee.
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    t.assert_spoke_usage_matches_positions();
    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 0.5, SeizeMode::Credit(0));
    assert!(receiver > 0, "credit mode opens a receiving account");
    t.assert_spoke_usage_matches_positions();

    // Bad-debt cleanup removes both sides of Dave's book: at one cent his
    // collateral is under the dust threshold and below his debt.
    t.set_price("USDC", usd_cents(1));
    t.clean_bad_debt_for(DAVE);
    t.assert_no_positions(DAVE);
    t.assert_spoke_usage_matches_positions();
}
