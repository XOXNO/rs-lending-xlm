//! S-08. A market that every supplier has left keeps the rounding cash that
//! no share owns. A borrow of that cash would leave debt without supply, so
//! it reverts with `PoolInsolvent` (INV-ACCT-09).

use controller::constants::RAY;
use soroban_sdk::vec;
use test_harness::helpers::HARNESS_HUB;
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, map_try_ok_unit, usdc_preset,
    LendingTest, MarketPreset, ALICE, BOB, CAROL,
};

/// Runs one market life to the end: one supplier, one borrower, one ledger,
/// then full repay, full withdraw and full revenue claim.
fn emptied_market(base_borrow_rate: i128) -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(MarketPreset {
            initial_liquidity: 0.0,
            ..usdc_preset()
        })
        .with_market(eth_preset())
        .with_market_params("USDC", |p| p.base_borrow_rate = base_borrow_rate)
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 1.0);
    t.borrow(BOB, "USDC", 500.0);
    t.advance_time(5);

    // The repay excess is refunded.
    t.repay(BOB, "USDC", 900.0);
    t.withdraw_all(ALICE, "USDC");
    t.claim_revenue("USDC");

    let s = t.pool_state_on_hub(HARNESS_HUB, "USDC");
    assert_eq!((s.supplied, s.borrowed, s.revenue), (0, 0, 0));
    assert!(s.cash > 0, "rounding leaves cash that no supplier owns");
    t
}

fn try_borrow_raw(
    t: &mut LendingTest,
    user: &str,
    asset: &str,
    amount: i128,
) -> Result<(), soroban_sdk::Error> {
    let account_id = t.resolve_account_id(user);
    let addr = t.get_or_create_user(user);
    let borrows = vec![&t.env, (hub_asset(t.resolve_asset(asset)), amount)];
    map_try_ok_unit(
        t.ctrl_client()
            .try_borrow(&addr, &account_id, &borrows, &None),
    )
}

#[test]
fn leftover_cash_cannot_be_borrowed_against_zero_supply() {
    for base_rate in [0, RAY / 100, RAY / 100_000] {
        let mut t = emptied_market(base_rate);
        let cash = t.pool_state_on_hub(HARNESS_HUB, "USDC").cash;

        assert_contract_error(
            try_borrow_raw(&mut t, BOB, "USDC", cash),
            errors::POOL_INSOLVENT,
        );
        assert_contract_error(
            try_borrow_raw(&mut t, BOB, "USDC", 1),
            errors::POOL_INSOLVENT,
        );

        let s = t.pool_state_on_hub(HARNESS_HUB, "USDC");
        assert_eq!((s.supplied, s.borrowed, s.cash), (0, 0, cash));
    }
}

/// The next supplier reopens the market. Borrows work again, and the
/// supplier can leave in full once the debt is repaid.
#[test]
fn a_new_supplier_reopens_the_emptied_market() {
    let mut t = emptied_market(RAY / 100);

    t.supply(CAROL, "USDC", 100.0);
    t.borrow(BOB, "USDC", 10.0);
    t.advance_time(5);
    t.repay(BOB, "USDC", 20.0);
    t.withdraw_all(CAROL, "USDC");

    let s = t.pool_state_on_hub(HARNESS_HUB, "USDC");
    assert_eq!((s.borrowed, s.supplied - s.revenue), (0, 0));
}
