use controller::constants::RAY;
use test_harness::{hub_asset, HubAssetKey,
    assert_contract_error, errors, eth_preset, usdc_preset, LendingTest, ALICE, BOB,
};
// Borrow gate

#[test]
fn test_borrow_above_max_utilization_rejected() {
    // Cap at 85 % (above default 80 % optimal — required by
    // `InterestRateModel::verify`'s `max >= optimal` invariant).
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_params("USDC", |p| {
            p.max_utilization_ray = RAY * 85 / 100;
        })
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);

    // Borrow $800 USDC → utilization = 80 % (allowed).
    t.borrow(BOB, "USDC", 800.0);

    // Borrow another $80 → utilization would be 88 %, > 85 %.
    let res = t.try_borrow(BOB, "USDC", 80.0);
    assert_contract_error(res, errors::UTILIZATION_ABOVE_MAX);
}

#[test]
fn test_borrow_at_max_utilization_succeeds() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_params("USDC", |p| {
            p.max_utilization_ray = RAY * 85 / 100;
        })
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);
    // Exactly at the cap — utilization == 85 %.
    t.borrow(BOB, "USDC", 850.0);
}

// Regression for Codex re-pass #1: the cap must use index-aware utilization
// (`borrowed * borrow_index / (supplied * supply_index)`), not the scaled
// ratio. After time passes, borrow_index grows faster than supply_index
// (the reserve factor diverts a slice of interest to revenue rather than
// suppliers), so indexed utilization rises above scaled utilization. A
// borrow that lands the pool at the scaled cap can still produce an
// indexed utilization above the cap once interest accrues.
#[test]
fn test_max_utilization_uses_index_aware_ratio() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_params("USDC", |p| {
            p.max_utilization_ray = RAY * 85 / 100;
        })
        .build();

    // Drive utilization to 80 % at indexes ≈ RAY (fresh pool). Bob
    // supplies an over-collateralised ETH stack so his LTV headroom is
    // huge and the test isolates the utilization-cap check from the
    // LTV check.
    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 1_000.0); // $2,000,000 collateral
    t.borrow(BOB, "USDC", 800.0);

    // Let interest accrue. Reserve factor diverts a slice of interest
    // to revenue, so borrow_index drifts above supply_index; indexed
    // utilization climbs above scaled utilization. After enough time
    // the indexed ratio passes the 85 % cap while the scaled ratio is
    // still ~80 %.
    t.advance_time(60 * 60 * 24 * 365 * 5); // 5 years
    t.update_indexes_for(&["USDC"]);

    // Any additional borrow that the *scaled-ratio* check would accept
    // must now be rejected because *index-aware* utilization is above
    // the cap.
    let result = t.try_borrow(BOB, "USDC", 1.0);
    assert_contract_error(result, errors::UTILIZATION_ABOVE_MAX);
}
// Withdraw gate

#[test]
fn test_withdraw_pushing_above_max_utilization_rejected() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_params("USDC", |p| {
            p.max_utilization_ray = RAY * 85 / 100; // 85 % cap.
        })
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 800.0); // 80 % utilization.

    // Alice withdraws $200 USDC: post-state supplied = 800, borrowed =
    // 800, util = 100 % → exceeds the 85 % cap.
    let res = t.try_withdraw(ALICE, "USDC", 200.0);
    assert_contract_error(res, errors::UTILIZATION_ABOVE_MAX);
}
// Zero-supply bypass regression
//
// A `cache.supplied == 0` short-circuit in `require_utilization_below_max`
// would assume accounting invariants make `borrowed == 0` too. A direct
// token donation to the pool's SAC address defeats that assumption: it
// inflates the live token balance so the reserve check on withdraw passes
// even though outstanding debt remains, letting the final supplier withdraw
// past the debt and leave the pool `(supplied = 0, borrowed > 0)` insolvent.
// Pins that the post-state insolvency guard panics instead.
#[test]
fn test_zero_supply_with_outstanding_borrow_rejected() {
    use test_harness::helpers;

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // ALICE supplies USDC, BOB borrows against ETH collateral.
    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 500.0);

    // Donation: mint USDC directly into the pool address, bypassing
    // any controller-tracked supply. Pool's SAC balance now exceeds
    // its accounting `supplied` by the donated amount.
    let pool_addr = t.resolve_market("USDC").pool.clone();
    let market = t.resolve_market("USDC");
    let donation_raw = helpers::f64_to_i128(10_000.0, market.decimals);
    market.token_admin.mint(&pool_addr, &donation_raw);

    // Attempt to fully withdraw using the `0` full-withdraw sentinel
    // so post-state `supplied` hits zero exactly. Reserve check
    // against the live token balance would pass (donation covers it),
    // but the post-state has supplied = 0 with borrowed > 0. The
    // utilization guard must reject this.
    use soroban_sdk::Vec as SorobanVec;
    let asset_addr = t.resolve_asset("USDC");
    let alice_addr = t.get_or_create_user(ALICE);
    let account_id = t.resolve_account_id(ALICE);
    let withdrawals: SorobanVec<(HubAssetKey, i128)> =
        soroban_sdk::vec![&t.env, (hub_asset(asset_addr), 0i128)];
    let ctrl = t.ctrl_client();
    let result = match ctrl.try_withdraw(&alice_addr, &account_id, &withdrawals, &None) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, errors::POOL_INSOLVENT);
}
// Admin-time validation

#[test]
fn test_update_params_rejects_max_below_optimal() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let model = controller::types::InterestRateModel {
        max_borrow_rate_ray: RAY,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY * 4 / 100,
        slope2_ray: RAY * 10 / 100,
        slope3_ray: RAY * 80 / 100,
        mid_utilization_ray: RAY * 50 / 100,
        optimal_utilization_ray: RAY * 80 / 100,
        // max < optimal — invalid.
        max_utilization_ray: RAY * 70 / 100,
        reserve_factor_bps: 1000,
    };
    let asset = t.resolve_asset("USDC");
    let result = t
        .ctrl_client()
        .try_upgrade_liquidity_pool_params(&hub_asset(asset), &model);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::INVALID_UTIL_RANGE);
}

#[test]
fn test_update_params_rejects_max_above_one() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let model = controller::types::InterestRateModel {
        max_borrow_rate_ray: RAY,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY * 4 / 100,
        slope2_ray: RAY * 10 / 100,
        slope3_ray: RAY * 80 / 100,
        mid_utilization_ray: RAY * 50 / 100,
        optimal_utilization_ray: RAY * 80 / 100,
        max_utilization_ray: RAY + 1, // > 100 % — invalid.
        reserve_factor_bps: 1000,
    };
    let asset = t.resolve_asset("USDC");
    let result = t
        .ctrl_client()
        .try_upgrade_liquidity_pool_params(&hub_asset(asset), &model);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::INVALID_UTIL_RANGE);
}
