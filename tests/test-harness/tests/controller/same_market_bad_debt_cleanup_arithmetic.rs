//! GH-30. An insolvent account holding supply and debt in the same market:
//! cleanup books the supply as protocol revenue and writes the whole debt off
//! against that market's suppliers. Pins the exact raw arithmetic that
//! ADR-0021 records as the netting follow-up.

use crate::shared::get_indexes;
use common::math::fp::Ray;
use controller::constants::RAY;
use test_harness::{hub_asset, usd, LendingTest, ALICE, BOB};

#[test]
fn cleanup_absorbs_same_market_supply_as_revenue_and_socializes_the_full_debt() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(BOB, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    // ALICE holds USDC supply and USDC debt, plus ETH debt that a price spike makes unpayable.
    t.supply(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "USDC", 500.0);
    t.borrow(ALICE, "ETH", 0.1);
    t.set_price("ETH", usd(20_000));
    let account = t.account_id(ALICE);
    let usdc = hub_asset(t.resolve_asset("USDC"));
    let (supplies, debts) = t.ctrl_client().get_account_positions(&account);
    let alice_supply_shares = supplies.get(usdc.clone()).unwrap().scaled_amount;
    let alice_debt_shares = debts.get(usdc.clone()).unwrap().scaled_amount;
    let before = t.pool_client("USDC").get_sync_data(&usdc).state;
    let (si_before, bi_before) = get_indexes(&t, "USDC");
    let bob_before = t.supply_balance_raw(BOB, "USDC");

    t.force_socialize_bad_debt_by_id(account);

    let after = t.pool_client("USDC").get_sync_data(&usdc).state;
    assert_eq!(
        after.revenue - before.revenue,
        alice_supply_shares,
        "the whole supply became revenue shares"
    );
    assert_eq!(
        before.borrowed - after.borrowed,
        alice_debt_shares,
        "the whole debt was burned"
    );
    let env = &t.env;
    let bad_debt = Ray::from(alice_debt_shares).mul_ceil(env, Ray::from(bi_before));
    let total_value = Ray::from(before.supplied).mul(env, Ray::from(si_before));
    let reduction = total_value
        .checked_sub(env, bad_debt.min(total_value))
        .div_floor(env, total_value);
    let expected_si = Ray::from(si_before)
        .mul_floor(env, reduction)
        .raw()
        .max(RAY / 1_000);
    let (si_after, _) = get_indexes(&t, "USDC");
    assert_eq!(
        si_after, expected_si,
        "the index write-down is the documented two-floor formula"
    );
    let bob_after = t.supply_balance_raw(BOB, "USDC");
    let bob_loss = bob_before - bob_after;
    assert!(bob_loss > 0);
    // Netting first would have written down nothing: 1_000 of supply covers 500 of debt.
    std::println!(
        "same-market cleanup: suppliers lost {bob_loss} raw, protocol booked {alice_supply_shares} raw shares"
    );
}
