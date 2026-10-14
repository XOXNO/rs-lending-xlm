use test_harness::{hub_asset, usdc_preset, LendingTest, ALICE, BOB};

#[test]
fn test_pool_claim_revenue_burns_supplied_ray_coverage() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);

    t.set_oracle_single_spot("USDC");

    t.supply(ALICE, "USDC", 1000.0);
    t.supply(BOB, "USDC", 1000.0);

    t.borrow(ALICE, "USDC", 500.0);

    t.advance_time(31_536_000);

    t.update_indexes_for(&["USDC"]);

    let rev = t.snapshot_revenue("USDC");
    assert!(rev > 0, "Expected some revenue after 1 year");

    let asset = t.resolve_market("USDC").asset.clone();
    let pool_addr = t.resolve_market("USDC").pool.clone();
    let tok = soroban_sdk::token::Client::new(&t.env, &asset);
    let pool_before = tok.balance(&pool_addr);
    let acc_before = tok.balance(&accumulator);

    let claimed = t.claim_revenue("USDC");
    assert_eq!(
        claimed, rev,
        "full-burn branch must claim the entire accrued revenue"
    );

    let rev_after = t.snapshot_revenue("USDC");
    assert_eq!(rev_after, 0);

    let pool_after = tok.balance(&pool_addr);
    let acc_after = tok.balance(&accumulator);
    assert_eq!(
        pool_before - pool_after,
        claimed,
        "pool must release exactly the claimed amount"
    );
    assert_eq!(
        acc_after - acc_before,
        claimed,
        "accumulator must receive exactly the claimed amount"
    );
}

/// Flipped by the pool claim policy fix (F8): with debt outstanding a claim draws only
/// the cash above the liquidation reserve, and pays nothing if that payout
/// would raise utilization past the 95% claim bound. After four more years of
/// accrual the reserve on the inflated supply exceeds the cash that was left
/// at the old buffer, so the claim pays nothing; a repayment that lifts the
/// cash above the reserve still leaves utilization near 100%, so the claim is
/// refused whole; once the borrower repays the rest, the whole revenue fits
/// and is paid. Before the fix the first claim paid all of the reserves and burned
/// revenue shares pro rata. A pro-rata burn under debt now needs revenue
/// above the cash over the reserve at low utilization (the pool unit tests
/// and `rv_pool_direct` pin it on seized-revenue books).
#[test]
fn test_pool_claim_revenue_refused_past_the_claim_bound_until_repayment_frees_room() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_max_utilization_disabled_all_markets()
        .build();

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);
    t.set_oracle_single_spot("USDC");

    t.supply(ALICE, "USDC", 1000.0);
    t.borrow(ALICE, "USDC", 700.0);

    t.advance_time(31_536_000);
    t.update_indexes_for(&["USDC"]);

    t.supply(BOB, "ETH", 1000.0);

    let usdc_key = hub_asset(t.resolve_asset("USDC"));
    let res = t.pool_client("USDC").get_reserves(&usdc_key);
    // Borrow reserves down to the liquidation buffer, the floor for a borrow.
    t.borrow_raw(BOB, "USDC", res - t.liquidation_buffer_raw("USDC"));

    // Reserves stay at the buffer while near-full utilization accrues revenue
    // past them.
    t.advance_time(31_536_000 * 4);
    t.update_indexes_for(&["USDC"]);

    let rev = t.snapshot_revenue("USDC");
    let usdc = t.resolve_asset("USDC");
    let res_raw = t.pool_client("USDC").get_reserves(&hub_asset(usdc));
    assert!(
        rev > res_raw,
        "Revenue {} must be > reserves {} to hit proportional burn",
        rev,
        res_raw
    );

    let asset = t.resolve_market("USDC").asset.clone();
    let pool_addr = t.resolve_market("USDC").pool.clone();
    let tok = soroban_sdk::token::Client::new(&t.env, &asset);
    let pool_before = tok.balance(&pool_addr);
    let acc_before = tok.balance(&accumulator);

    // The reserve grew with the supply while the cash did not.
    let reserve = t.liquidation_buffer_raw("USDC");
    assert!(
        reserve > res_raw,
        "reserve {reserve} must exceed reserves {res_raw}"
    );
    assert_eq!(
        t.claim_revenue("USDC"),
        0,
        "nothing above the reserve to pay"
    );
    assert_eq!(tok.balance(&pool_addr), pool_before);
    assert_eq!(tok.balance(&accumulator), acc_before);
    assert_eq!(t.snapshot_revenue("USDC"), rev);

    // BOB repays enough of his debt (which compounded for four years) to lift
    // the cash to the reserve plus half the revenue.
    // Paying that half would still leave utilization near 100%, past the 95%
    // claim bound, so the claim is refused whole: it pays nothing.
    let target = reserve + rev / 2;
    t.repay_raw(BOB, "USDC", target - res_raw);
    let res_raw = t
        .pool_client("USDC")
        .get_reserves(&hub_asset(t.resolve_asset("USDC")));
    let reserve = t.liquidation_buffer_raw("USDC");
    let above_reserve = res_raw - reserve;
    assert!(above_reserve > 0 && t.snapshot_revenue("USDC") > above_reserve);
    assert!(
        t.claim_utilization_room_raw("USDC") < above_reserve,
        "fixture: the payout would cross the 95% line"
    );
    let pool_before = tok.balance(&pool_addr);
    assert_eq!(t.claim_revenue("USDC"), 0, "refused whole, no revert");
    assert_eq!(tok.balance(&pool_addr), pool_before);
    assert_eq!(tok.balance(&accumulator), acc_before);

    // BOB repays the rest: utilization collapses and the whole revenue fits
    // under the line, so the claim pays it and burns every revenue share.
    let bob_debt = t.borrow_balance_raw(BOB, "USDC");
    t.repay_raw(BOB, "USDC", bob_debt + 1_000_000);
    let res_raw = t
        .pool_client("USDC")
        .get_reserves(&hub_asset(t.resolve_asset("USDC")));
    let reserve = t.liquidation_buffer_raw("USDC");
    let rev = t.snapshot_revenue("USDC");
    assert!(rev <= res_raw - reserve && rev <= t.claim_utilization_room_raw("USDC"));
    let pool_before = tok.balance(&pool_addr);

    let claimed = t.claim_revenue("USDC");

    assert!(claimed > 0, "the claim pays once it fits the bound");
    assert_eq!(claimed, rev, "the whole floored revenue is paid");

    let pool_after = tok.balance(&pool_addr);
    let acc_after = tok.balance(&accumulator);
    assert_eq!(
        pool_before - pool_after,
        claimed,
        "pool must release exactly the claimed amount"
    );
    assert_eq!(
        acc_after - acc_before,
        claimed,
        "accumulator must receive exactly the claimed amount"
    );
    assert!(
        t.pool_client("USDC")
            .get_utilisation(&hub_asset(t.resolve_asset("USDC")))
            <= 950_000_000_000_000_000_000_000_000,
        "the claim leaves utilization within the claim bound"
    );
}
