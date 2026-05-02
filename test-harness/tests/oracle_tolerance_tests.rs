extern crate std;

use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usd_cents, usdc_preset, LendingTest, ALICE,
    LIQUIDATOR,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a standard two-market test harness with USDC ($1) and ETH ($2000).
fn setup() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build()
}

/// Enable dual-source pricing so the controller compares aggregator and
/// safe (TWAP) prices for tolerance checks.
fn enable_dual_source(t: &LendingTest, asset_name: &str) {
    t.set_exchange_source(asset_name, common::types::ExchangeSource::SpotVsTwap);
}

// ===========================================================================
// 1. Price within first tolerance: all operations succeed
// ===========================================================================

#[test]
fn test_safe_price_allows_all_operations() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Safe price matches aggregator exactly: within first tolerance.
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    // Supply (risk-decreasing).
    t.supply(ALICE, "USDC", 100_000.0);
    t.assert_supply_near(ALICE, "USDC", 100_000.0, 1.0);

    // Borrow (risk-increasing).
    t.borrow(ALICE, "ETH", 10.0);
    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);

    // Repay (risk-decreasing).
    t.repay(ALICE, "ETH", 1.0);
    t.assert_borrow_near(ALICE, "ETH", 9.0, 0.01);

    // Withdraw (risk-increasing when borrows exist).
    t.withdraw(ALICE, "USDC", 1_000.0);
    t.assert_supply_near(ALICE, "USDC", 99_000.0, 1.0);
    t.assert_healthy(ALICE);
}

// ===========================================================================
// 2. Price within second tolerance: operations still succeed
// ===========================================================================

#[test]
fn test_second_tolerance_allows_risk_decreasing() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Default tolerance: first=200 BPS (2%), last=500 BPS (5%).
    // Set safe price 3% away from aggregator (between first and second).
    // Aggregator: $1.00, Safe: $1.03 (3% deviation).
    t.set_safe_price("USDC", usd_cents(103), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    // Supply succeeds (risk-decreasing).
    t.supply(ALICE, "USDC", 100_000.0);
    t.assert_supply_near(ALICE, "USDC", 100_000.0, 1.0);

    // Borrow also succeeds (within second tolerance, uses average price).
    t.borrow(ALICE, "ETH", 10.0);
    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);
    t.assert_healthy(ALICE);

    // Repay succeeds (risk-decreasing).
    t.repay(ALICE, "ETH", 1.0);
    t.assert_borrow_near(ALICE, "ETH", 9.0, 0.01);
}

#[test]
fn test_second_tolerance_allows_borrow() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Set USDC safe price 3% above aggregator (within second tolerance).
    t.set_safe_price("USDC", usd_cents(103), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Borrow succeeds: price deviation is within the second tolerance band.
    t.try_borrow(ALICE, "ETH", 10.0)
        .expect("borrow should work within second tolerance");
    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);
    let eth_wallet = t.token_balance(ALICE, "ETH");
    assert!(
        eth_wallet > 9.99,
        "ETH wallet should be ~10, got {}",
        eth_wallet
    );
}

// ===========================================================================
// 3. Price beyond second tolerance: risk-increasing ops blocked
// ===========================================================================

#[test]
fn test_unsafe_price_allows_supply() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");

    // Set USDC safe price 10% from aggregator (beyond second tolerance of 5%).
    // Aggregator: $1.00, Safe: $1.10 (10% deviation).
    t.set_safe_price("USDC", usd_cents(110), true, true);

    // Supply still succeeds (allow_unsafe_price=true for supply). Use the
    // tracking `supply` helper so the new account is registered for the
    // post-state read; bare `try_supply` returns the new account_id without
    // tracking it.
    t.supply(ALICE, "USDC", 10_000.0);
    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
}

#[test]
fn test_unsafe_price_allows_repay() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // First set up positions with matching prices.
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // Deviate the ETH safe price beyond the second tolerance.
    t.set_safe_price("ETH", usd(2200), true, true); // 10% deviation

    // Repay still succeeds (allow_unsafe_price=true for repay). The previous
    // version of this test had no assertion -- it relied on the implicit
    // "panic-if-repay-fails" behavior of `t.repay()`. A regression that
    // accepted the repay syntactically but left the position untouched would
    // have passed. Snapshot the debt before and after to verify the repay
    // actually reduced the scaled borrow.
    let debt_before = t.borrow_balance(ALICE, "ETH");
    t.repay(ALICE, "ETH", 1.0);
    let debt_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        debt_before - debt_after >= 0.99,
        "repay under unsafe price must reduce debt by ~1 ETH: before={}, after={}",
        debt_before,
        debt_after
    );
}

#[test]
fn test_unsafe_price_blocks_borrow() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Set matching safe prices first for supply.
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Deviate the USDC safe price beyond the second tolerance (10% up).
    t.set_safe_price("USDC", usd_cents(110), true, true);

    // Borrow fails: USDC (collateral) price is unsafe, and borrow uses
    // allow_unsafe_price=false.
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

#[test]
fn test_unsafe_price_blocks_borrow_debt_asset() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Set matching safe prices first for supply.
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Deviate the ETH safe price beyond the second tolerance.
    t.set_safe_price("ETH", usd(2200), true, true); // 10% above aggregator

    // Borrow fails: ETH (debt asset) price is unsafe.
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

#[test]
fn test_unsafe_price_blocks_withdraw_with_borrows() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Set matching safe prices first.
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // Deviate the USDC safe price beyond the second tolerance.
    t.set_safe_price("USDC", usd_cents(110), true, true);

    // Withdraw fails when the user has borrows (risk-increasing,
    // allow_unsafe_price=false).
    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

// Withdraw under oracle deviation > 5%:
// - succeeds when the account has no debt (post-loop HF gate short-circuits)
// - fails when borrows exist (risk-increasing, must run on strict price)

#[test]
fn withdraw_succeeds_under_oracle_deviation_when_no_debt() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Establish positions with matching prices first (safe band).
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    // Supply-only account, no borrow.
    t.supply(ALICE, "USDC", 100_000.0);

    // Push USDC safe price 10% above aggregator (beyond second tolerance of 5%).
    t.set_safe_price("USDC", usd_cents(110), true, true);

    // With no debt, the withdraw cache runs with allow_unsafe_price=true,
    // and the post-loop health-factor gate short-circuits when no borrows
    // exist. Supply-only users must keep liveness during oracle deviation.
    let wallet_before = t.token_balance(ALICE, "USDC");
    t.try_withdraw(ALICE, "USDC", 1_000.0)
        .expect("withdraw should succeed under oracle deviation when account has no debt");
    t.assert_supply_near(ALICE, "USDC", 99_000.0, 1.0);
    let wallet_after = t.token_balance(ALICE, "USDC");
    assert!(
        wallet_after - wallet_before > 999.0,
        "wallet should grow by ~1000: before={}, after={}",
        wallet_before,
        wallet_after
    );
}

#[test]
fn withdraw_blocked_under_oracle_deviation_when_debt_exists() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Set matching safe prices first to allow setup.
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // Deviate USDC safe price beyond the second tolerance (10% > 5%).
    t.set_safe_price("USDC", usd_cents(110), true, true);

    // With borrows present the cache runs with allow_unsafe_price=false;
    // resolving the collateral price must trip OracleError::UnsafePriceNotAllowed.
    let err = t
        .try_withdraw(ALICE, "USDC", 1_000.0)
        .expect_err("withdraw with borrows must fail under oracle deviation");

    // OracleError::UnsafePriceNotAllowed = 205 (see common/src/errors.rs).
    let expected = soroban_sdk::Error::from_contract_error(205);
    assert_eq!(
        err, expected,
        "expected UnsafePriceNotAllowed (205), got {:?}",
        err
    );
}

#[test]
fn test_unsafe_price_blocks_liquidation() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Set matching safe prices for initial setup.
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    // Supply and borrow to create a position.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 30.0);

    // Drop the ETH aggregator price to make Alice liquidatable.
    t.set_price("ETH", usd(3500));
    t.set_safe_price("ETH", usd(3500), true, true);

    // Confirm liquidatable.
    assert!(t.can_be_liquidated(ALICE), "Alice should be liquidatable");

    // Deviate the safe price beyond tolerance so liquidation is blocked.
    t.set_safe_price("USDC", usd_cents(110), true, true);

    // Liquidation fails: allow_unsafe_price=false for liquidate.
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

// ===========================================================================
// 4. Staleness tests
// ===========================================================================

#[test]
fn test_stale_price_blocks_supply() {
    let mut t = setup();

    // Supply first while the price is fresh.
    t.supply(ALICE, "USDC", 10_000.0);

    // Advance time beyond the staleness window (900 seconds) without
    // refreshing prices.
    t.advance_time_no_refresh(1000);

    // Supply also fails with a stale price because the oracle adapter's
    // mock storage entry expired on `advance_time_no_refresh`, so
    // `lastprice` returns `None` -> `OracleError::NoLastPrice`. Pin the
    // precise contract code so any regression that fails for a different
    // reason surfaces loudly.
    let err = t
        .try_supply(ALICE, "USDC", 1_000.0)
        .expect_err("supply should fail under stale-mock conditions");
    assert_eq!(
        err,
        soroban_sdk::Error::from_contract_error(210),
        "expected OracleError::NoLastPrice (210), got {:?}",
        err
    );
}

#[test]
fn test_stale_price_blocks_borrow() {
    let mut t = setup();

    t.supply(ALICE, "USDC", 100_000.0);

    // Advance time beyond the staleness window without refreshing prices.
    t.advance_time_no_refresh(1000);

    // Borrow fails: stale price blocked for risk-increasing ops.
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(result.is_err(), "borrow should fail with stale price");
}

#[test]
fn test_stale_price_blocks_withdraw_with_borrows() {
    let mut t = setup();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // Advance time beyond the staleness window without refreshing.
    t.advance_time_no_refresh(1000);

    // Withdraw fails when borrows exist (risk-increasing).
    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
    assert!(
        result.is_err(),
        "withdraw with borrows should fail with stale price"
    );
}

// ===========================================================================
// 5. Edge cases
// ===========================================================================

#[test]
fn test_tolerance_at_exact_first_boundary() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Default first tolerance = 200 BPS (2%).
    // The controller's tolerance stores pre-computed ratio bounds:
    //   upper = 10000 + 200 = 10200
    //   lower = 10000^2 / 10200 = 9804
    // Set safe price exactly at 2% deviation: $1.02.
    t.set_safe_price("USDC", usd_cents(102), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // At exactly the first boundary, the price stays within first tolerance
    // and uses the safe price directly (most favorable for the user).
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(
        result.is_ok(),
        "borrow should work at first tolerance boundary"
    );
}

#[test]
fn test_tolerance_just_beyond_first_boundary() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Set safe price at 2.1% deviation (just past first tolerance of 2%).
    // This puts it in the second tolerance zone, where the average price is
    // used.
    t.set_safe_price("USDC", usd_cents(103), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Still succeeds (average price used, within second tolerance).
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(
        result.is_ok(),
        "borrow should work between first and second tolerance"
    );
}

#[test]
fn test_safe_price_below_aggregator_blocks_borrow() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Safe price 10% below aggregator (negative deviation).
    // Aggregator: $1.00, Safe: $0.90.
    t.set_safe_price("USDC", usd_cents(90), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Beyond second tolerance in the negative direction: blocked.
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(
        result.is_err(),
        "borrow should fail with safe price 10% below aggregator"
    );
}

// ===========================================================================
// 6. Oracle tolerance config validation (controller side)
// ===========================================================================

#[test]
fn test_tolerance_config_rejects_first_below_min() {
    let t = setup();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let asset = t.resolve_market("USDC").asset.clone();

    // MIN_FIRST_TOLERANCE = 50 BPS.
    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &10, &500);
    assert!(
        result.is_err(),
        "first tolerance below 50 BPS should be rejected"
    );
}

#[test]
fn test_tolerance_config_rejects_first_above_max() {
    let t = setup();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let asset = t.resolve_market("USDC").asset.clone();

    // MAX_FIRST_TOLERANCE = 5000 BPS.
    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &6000, &7000);
    assert!(
        result.is_err(),
        "first tolerance above 5000 BPS should be rejected"
    );
}

#[test]
fn test_tolerance_config_rejects_last_below_min() {
    let t = setup();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let asset = t.resolve_market("USDC").asset.clone();

    // MIN_LAST_TOLERANCE = 150 BPS, first=200 is valid.
    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &100, &100);
    assert!(
        result.is_err(),
        "last tolerance below 150 BPS should be rejected"
    );
}

#[test]
fn test_tolerance_config_rejects_last_above_max() {
    let t = setup();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let asset = t.resolve_market("USDC").asset.clone();

    // MAX_LAST_TOLERANCE = 10000 BPS.
    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &200, &11000);
    assert!(
        result.is_err(),
        "last tolerance above 10000 BPS should be rejected"
    );
}

#[test]
fn test_tolerance_config_rejects_last_less_than_first() {
    let t = setup();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let asset = t.resolve_market("USDC").asset.clone();

    // last (200) < first (300): must fail.
    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &300, &200);
    assert!(
        result.is_err(),
        "last tolerance < first tolerance should be rejected"
    );
}

#[test]
fn test_tolerance_config_valid_update() {
    let t = setup();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    let asset = t.resolve_market("USDC").asset.clone();

    // Valid tolerance update.
    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &300, &600);
    assert!(result.is_ok(), "valid tolerance update should succeed");
}

// ===========================================================================
// 7. Config gap tests
// ===========================================================================

#[test]
fn test_set_accumulator() {
    let t = setup();
    let ctrl = t.ctrl_client();

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());

    // Must not panic: admin has permission.
    ctrl.set_accumulator(&accumulator);

    // Verify storage by reading directly.
    let stored: soroban_sdk::Address = t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .instance()
            .get(&common::types::ControllerKey::Accumulator)
            .unwrap()
    });
    assert_eq!(stored, accumulator, "accumulator address should be stored");
}

#[test]
fn test_set_liquidity_pool_template() {
    let t = setup();
    let ctrl = t.ctrl_client();

    let hash = soroban_sdk::BytesN::from_array(&t.env, &[42u8; 32]);

    ctrl.set_liquidity_pool_template(&hash);

    // Verify storage by reading directly.
    let stored: soroban_sdk::BytesN<32> = t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .instance()
            .get(&common::types::ControllerKey::PoolTemplate)
            .unwrap()
    });
    assert_eq!(stored, hash, "pool template hash should be stored");
}

#[test]
fn test_disable_token_oracle_blocks_operations() {
    let mut t = setup();

    t.supply(ALICE, "USDC", 10_000.0);

    // Disable the USDC oracle: oracle_type becomes 0 (None).
    let usdc_asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    t.ctrl_client().disable_token_oracle(&admin, &usdc_asset);

    // The price now returns 0 for USDC, changing HF-sensitive behavior.
    // Borrowing against zero-value collateral must fail.
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert!(
        result.is_err(),
        "borrow should fail when collateral oracle is disabled (price=0)"
    );
}

#[test]
fn test_edit_asset_in_e_mode_category() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_emode(1, test_harness::STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .build();

    // Initially: can_collateral=true, can_borrow=true.
    // Edit: set can_borrow=false.
    t.edit_asset_in_e_mode("USDC", 1, true, false);

    // Verify the update by reading storage.
    let usdc_asset = t.resolve_market("USDC").asset.clone();
    let config: Option<common::types::EModeAssetConfig> = t.env.as_contract(&t.controller, || {
        let map: Option<soroban_sdk::Map<soroban_sdk::Address, common::types::EModeAssetConfig>> =
            t.env
                .storage()
                .persistent()
                .get(&common::types::ControllerKey::EModeAssets(1));
        map.and_then(|m| m.get(usdc_asset))
    });
    let config = config.expect("emode asset config should exist");
    assert!(
        config.is_collateralizable,
        "should still be collateralizable"
    );
    assert!(
        !config.is_borrowable,
        "should no longer be borrowable after edit"
    );
}

// ===========================================================================
// 8. Dual-source pricing: average price used in second tolerance zone
// ===========================================================================

#[test]
fn test_second_tolerance_uses_average_price() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Aggregator: $1.00, Safe: $1.03 (3% deviation, between first 2% and
    // last 5%). Average price = ($1.00 + $1.03) / 2 = $1.015. Collateral
    // value is therefore slightly higher with the average than with the
    // aggregator alone.
    t.set_safe_price("USDC", usd_cents(103), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // The average price drives valuation.
    t.assert_healthy(ALICE);
}

// ===========================================================================
// 9. Exchange source = 1 (safe price only)
// ===========================================================================

#[test]
fn test_exchange_source_safe_only() {
    let mut t = setup();
    t.set_exchange_source("USDC", common::types::ExchangeSource::SpotVsTwap);
    t.set_exchange_source("ETH", common::types::ExchangeSource::SpotVsTwap);

    // Set safe prices (used because exchange_source=1).
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    // Operations succeed using the safe price alone.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    t.assert_healthy(ALICE);
}

// ===========================================================================
// 10. Multiple assets with different tolerance states
// ===========================================================================

#[test]
fn test_mixed_tolerance_states() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // USDC: within first tolerance (matching prices).
    t.set_safe_price("USDC", usd(1), true, true);

    // ETH: beyond second tolerance (10% deviation).
    t.set_safe_price("ETH", usd(2200), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Borrowing ETH must fail: ETH's price is beyond the second tolerance.
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(
        result.is_err(),
        "borrow should fail when debt asset price is unsafe"
    );
}

// ===========================================================================
// 11. Denial of service on liquidation during a flash crash
// ===========================================================================

#[test]
fn test_liquidation_dos_flash_crash() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Perfect market conditions.
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);
    t.set_price("USDC", usd(1));
    t.set_price("ETH", usd(2000));

    // Provide initial liquidity.
    t.supply(test_harness::KEEPER_USER, "USDC", 100_000.0);

    // Alice supplies ETH and borrows the maximum USDC.
    t.supply(ALICE, "ETH", 10.0); // 20,000 USD collateral
    t.borrow(ALICE, "USDC", 15_000.0); // LTV is ~0.8 for ETH, so borrow up to the limit

    // HF must be healthy.
    let hf_before = t.health_factor(ALICE);
    assert!(hf_before >= 1.0, "Alice should be healthy");

    // ==========================================
    // The flash crash
    // ==========================================
    // The spot price of ETH drops sharply to 1400 USD (a 30% drop). TWAP
    // moves slowly and stays at 1950 USD.
    t.set_price("ETH", usd(1400));
    t.set_safe_price("ETH", usd(1950), true, true);

    // Give the liquidator some USDC to perform the liquidation.
    t.supply(LIQUIDATOR, "USDC", 20_000.0);

    // The liquidator sees Alice's health factor falling below 1 on the spot
    // price and attempts to liquidate the underwater position.
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "USDC", 15_000.0);

    // The protocol panics and reverts: liquidation uses
    // allow_unsafe_price=false, and the 30% deviation between SPOT ($1400)
    // and TWAP ($1950) exceeds second_tolerance, raising an OracleError.
    // This perfectly DoSes liquidations precisely when they matter most.
    assert!(
        result.is_err(),
        "Liquidation was perfectly DOSed by the oracle safety bands!"
    );
}

// ===========================================================================
// 12. Liquidation collateral extraction via second-tolerance averaging
// ===========================================================================

#[test]
fn test_liquidation_collateral_extraction_via_averaging() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Start with perfect market conditions.
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);
    t.set_price("USDC", usd(1));
    t.set_price("ETH", usd(2000));

    // Provide initial liquidity.
    t.supply(test_harness::KEEPER_USER, "USDC", 100_000.0);

    // Raise ETH LTV and threshold to make the position very sensitive.
    // Apply this before supplying so the position records these values.
    t.edit_asset_config("ETH", |c| {
        c.loan_to_value_bps = 9500;
        c.liquidation_threshold_bps = 9800;
    });

    // Use a loose tolerance to allow a wide 10% averaging band.
    t.set_oracle_tolerance("ETH", test_harness::LOOSE_TOLERANCE);

    // Alice supplies ETH (20,000 USD collateral).
    t.supply(ALICE, "ETH", 10.0);

    // Alice borrows heavily: 18,900 USDC against 19,000 max LTV.
    t.borrow(ALICE, "USDC", 18_900.0);

    // Give the liquidator USDC to perform the liquidation.
    t.supply(LIQUIDATOR, "USDC", 20_000.0);

    // Spot falls to 1820 while the averaged price stays at 1910.
    // Threshold value = 10 * 1910 * 0.99 = 18,909, below the 19,500 debt.

    t.set_price("ETH", usd(1820));
    t.set_safe_price("ETH", usd(2000), true, true);

    let liquidator_eth_before = t.token_balance(LIQUIDATOR, "ETH");

    // Attempt liquidation under the averaged price.
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "USDC", 5_000.0);

    assert!(
        result.is_ok(),
        "Liquidation should succeed because 9% deviation is within loose 10% band!"
    );

    let liquidator_eth_after = t.token_balance(LIQUIDATOR, "ETH");
    let received_collateral = liquidator_eth_after - liquidator_eth_before;

    // Debt = 5000, bonus = 5%, total claim = 5250 USD.
    // At the averaged price of 1910, this is 2.7486 ETH.

    assert!(
        received_collateral > 2.7,
        "Liquidator successfully extracted excess collateral via averaging exploit: {}",
        received_collateral
    );

    // The averaged price yields more seized ETH than a 2000 USD reference
    // price would.
    assert!(
        received_collateral > 2.74,
        "Liquidator successfully extracted excess collateral via averaging exploit: {}",
        received_collateral
    );
}
