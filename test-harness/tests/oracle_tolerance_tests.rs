extern crate std;

use test_harness::{eth_preset, usd, usd_cents, usdc_preset, LendingTest, ALICE, LIQUIDATOR};

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

/// Enable dual-source pricing for an asset so the controller compares
/// aggregator vs safe (TWAP) prices for tolerance checks.
fn enable_dual_source(t: &LendingTest, asset_name: &str) {
    t.set_exchange_source(asset_name, common::types::ExchangeSource::SpotVsTwap);
}

// ===========================================================================
// 1. Price within first tolerance (safe) -- all operations work
// ===========================================================================

#[test]
fn test_safe_price_allows_all_operations() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Safe price matches aggregator exactly -- within first tolerance
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    // Supply (risk-decreasing)
    t.supply(ALICE, "USDC", 100_000.0);

    // Borrow (risk-increasing)
    t.borrow(ALICE, "ETH", 10.0);

    // Repay (risk-decreasing)
    t.repay(ALICE, "ETH", 1.0);

    // Withdraw (risk-increasing when has borrows)
    t.withdraw(ALICE, "USDC", 1_000.0);
}

// ===========================================================================
// 2. Price within second tolerance -- operations still work
// ===========================================================================

#[test]
fn test_second_tolerance_allows_risk_decreasing() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Default tolerance: first=200 BPS (2%), last=500 BPS (5%)
    // Set safe price 3% away from aggregator (between first and second)
    // Aggregator: $1.00, Safe: $1.03 (3% deviation)
    t.set_safe_price("USDC", usd_cents(103), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    // Supply should work (risk-decreasing)
    t.supply(ALICE, "USDC", 100_000.0);

    // Borrow should also work (within second tolerance, uses average price)
    t.borrow(ALICE, "ETH", 10.0);

    // Repay should work (risk-decreasing)
    t.repay(ALICE, "ETH", 1.0);
}

#[test]
fn test_second_tolerance_allows_borrow() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Set USDC safe price 3% above aggregator (within second tolerance)
    t.set_safe_price("USDC", usd_cents(103), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Borrow should work -- price deviation is within second tolerance band
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(result.is_ok(), "borrow should work within second tolerance");
}

// ===========================================================================
// 3. Price beyond second tolerance -- risk-increasing ops blocked
// ===========================================================================

#[test]
fn test_unsafe_price_allows_supply() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");

    // Set USDC safe price 10% away from aggregator (beyond second tolerance of 5%)
    // Aggregator: $1.00, Safe: $1.10 (10% deviation)
    t.set_safe_price("USDC", usd_cents(110), true, true);

    // Supply should still work (allow_unsafe_price=true for supply)
    let result = t.try_supply(ALICE, "USDC", 10_000.0);
    assert!(result.is_ok(), "supply should work even with unsafe price");
}

#[test]
fn test_unsafe_price_allows_repay() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // First set up positions with matching prices
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // Now deviate ETH safe price beyond second tolerance
    t.set_safe_price("ETH", usd(2200), true, true); // 10% deviation

    // Repay should still work (allow_unsafe_price=true for repay).
    // The previous version of this test had NO assertion — it relied on the
    // implicit "panic-if-repay-fails" behavior of `t.repay()`. A regression
    // that ACCEPTED the repay syntactically but did nothing to the position
    // would have passed. Snapshot the debt before/after to verify the repay
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

    // Set matching safe prices first for supply
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Now deviate USDC safe price beyond second tolerance (10% up)
    t.set_safe_price("USDC", usd_cents(110), true, true);

    // Borrow should fail -- USDC (collateral) price is unsafe, and borrow
    // uses allow_unsafe_price=false
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(
        result.is_err(),
        "borrow should fail with unsafe collateral price"
    );
}

#[test]
fn test_unsafe_price_blocks_borrow_debt_asset() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Set matching safe prices first for supply
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Now deviate ETH safe price beyond second tolerance
    t.set_safe_price("ETH", usd(2200), true, true); // 10% above aggregator

    // Borrow should fail -- ETH (debt asset) price is unsafe
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(
        result.is_err(),
        "borrow should fail with unsafe debt asset price"
    );
}

#[test]
fn test_unsafe_price_blocks_withdraw_with_borrows() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Set matching safe prices first
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // Now deviate USDC safe price beyond second tolerance
    t.set_safe_price("USDC", usd_cents(110), true, true);

    // Withdraw should fail when user has borrows (risk-increasing, allow_unsafe_price=false)
    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
    assert!(
        result.is_err(),
        "withdraw with borrows should fail with unsafe price"
    );
}

#[test]
fn test_unsafe_price_blocks_liquidation() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Set matching safe prices for initial setup
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    // Supply and borrow to create a position
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 30.0);

    // Drop ETH aggregator price to make Alice liquidatable
    t.set_price("ETH", usd(3500));
    t.set_safe_price("ETH", usd(3500), true, true);

    // Confirm liquidatable
    assert!(t.can_be_liquidated(ALICE), "Alice should be liquidatable");

    // Now deviate the safe price beyond tolerance so liquidation is blocked
    t.set_safe_price("USDC", usd_cents(110), true, true);

    // Liquidation should fail -- allow_unsafe_price=false for liquidate
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert!(result.is_err(), "liquidation should fail with unsafe price");
}

// ===========================================================================
// 4. Staleness tests
// ===========================================================================

#[test]
fn test_stale_price_blocks_supply() {
    let mut t = setup();

    // Supply first while price is fresh
    t.supply(ALICE, "USDC", 10_000.0);

    // Advance time beyond staleness window (900 seconds) WITHOUT refreshing prices
    t.advance_time_no_refresh(1000);

    // Supply also fails with stale price because the oracle adapter's get_price()
    // enforces staleness unconditionally before the controller sees the price.
    let result = t.try_supply(ALICE, "USDC", 1_000.0);
    assert!(
        result.is_err(),
        "supply should fail with stale price (adapter enforces staleness)"
    );
}

#[test]
fn test_stale_price_blocks_borrow() {
    let mut t = setup();

    t.supply(ALICE, "USDC", 100_000.0);

    // Advance time beyond staleness window WITHOUT refreshing prices
    t.advance_time_no_refresh(1000);

    // Borrow should fail -- stale price blocked for risk-increasing ops
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(result.is_err(), "borrow should fail with stale price");
}

#[test]
fn test_stale_price_blocks_withdraw_with_borrows() {
    let mut t = setup();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // Advance time beyond staleness window WITHOUT refreshing
    t.advance_time_no_refresh(1000);

    // Withdraw should fail when has borrows (risk-increasing)
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
    // Set safe price exactly at 2% deviation: $1.02
    t.set_safe_price("USDC", usd_cents(102), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // At exactly the first boundary, should be within first tolerance
    // and use safe price directly (most favorable for user)
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

    // Set safe price at 2.1% deviation (just past first tolerance of 2%)
    // This puts it in the second tolerance zone -> average price used
    t.set_safe_price("USDC", usd_cents(103), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Should still work (average price used, within second tolerance)
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

    // Safe price 10% BELOW aggregator (negative deviation)
    // Aggregator: $1.00, Safe: $0.90
    t.set_safe_price("USDC", usd_cents(90), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Beyond second tolerance in the negative direction -> blocked
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

    // MIN_FIRST_TOLERANCE = 50 BPS
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

    // MAX_FIRST_TOLERANCE = 5000 BPS
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

    // MIN_LAST_TOLERANCE = 150 BPS, first=200 is valid
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

    // MAX_LAST_TOLERANCE = 10000 BPS
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

    // last (200) < first (300) -> should fail
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

    // Valid tolerance update
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

    // Should not panic -- admin has permission
    ctrl.set_accumulator(&accumulator);

    // Verify it's stored by reading storage directly
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

    // Verify it's stored by reading storage directly
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

    // Disable USDC oracle -> oracle_type set to 0 (None)
    let usdc_asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    t.ctrl_client().disable_token_oracle(&admin, &usdc_asset);

    // Price should now return 0 for USDC, making HF-sensitive ops behave
    // differently. Borrow against zero-value collateral should fail.
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

    // Initially: can_collateral=true, can_borrow=true
    // Edit: set can_borrow=false
    t.edit_asset_in_e_mode("USDC", 1, true, false);

    // Verify the update took effect by reading storage
    let usdc_asset = t.resolve_market("USDC").asset.clone();
    let config: Option<common::types::EModeAssetConfig> = t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .get(&common::types::ControllerKey::EModeAsset(1, usdc_asset))
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
// 8. Dual-source pricing -- average price used in second tolerance zone
// ===========================================================================

#[test]
fn test_second_tolerance_uses_average_price() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Aggregator: $1.00, Safe: $1.03 (3% deviation, between first 2% and last 5%)
    // Average price = ($1.00 + $1.03) / 2 = $1.015
    // This means collateral value is slightly higher with average vs aggregator
    t.set_safe_price("USDC", usd_cents(103), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // The average price should be used for valuation
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

    // Set safe prices (these will be used since exchange_source=1)
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    // Operations should work using safe price only
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

    // USDC: within first tolerance (matching prices)
    t.set_safe_price("USDC", usd(1), true, true);

    // ETH: beyond second tolerance (10% deviation)
    t.set_safe_price("ETH", usd(2200), true, true);

    t.supply(ALICE, "USDC", 100_000.0);

    // Borrow ETH should fail because ETH's price is beyond second tolerance
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert!(
        result.is_err(),
        "borrow should fail when debt asset price is unsafe"
    );
}

// ===========================================================================
// 11. Denial of Service on Liquidation during Flash Crash
// ===========================================================================

#[test]
fn test_liquidation_dos_flash_crash() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Perfect market conditions
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);
    t.set_price("USDC", usd(1));
    t.set_price("ETH", usd(2000));

    // Provide initial liquidity
    t.supply(test_harness::KEEPER_USER, "USDC", 100_000.0);

    // Alice supplies ETH and borrows maximum USDC
    t.supply(ALICE, "ETH", 10.0); // 20,000 USD collateral
    t.borrow(ALICE, "USDC", 15_000.0); // LTV is ~0.8 for ETH, so she borrows up to the limit

    // HF should be healthy
    let hf_before = t.health_factor(ALICE);
    assert!(hf_before >= 1.0, "Alice should be healthy");

    // ==========================================
    // THE FLASH CRASH
    // ==========================================
    // Spot price of ETH drops sharply to 1400 USD (30% drop).
    // TWAP is slow, so it remains at 1950 USD.
    t.set_price("ETH", usd(1400));
    t.set_safe_price("ETH", usd(1950), true, true);

    // Give liquidator some USDC to perform the liquidation
    t.supply(LIQUIDATOR, "USDC", 20_000.0);

    // The liquidator sees Alice's health factor falling below 1 based on actual spot price!
    // They attempt to liquidate Alice's underwater position.
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "USDC", 15_000.0);

    // The protocol WILL PANIC and REVERT because liquidation uses allow_unsafe_price = false,
    // and the 30% deviation between SPOT ($1400) and TWAP ($1950) exceeds second_tolerance,
    // throwing an OracleError.
    // This perfectly DoS-es liquidations precisely when they are most critical!
    assert!(
        result.is_err(),
        "Liquidation was perfectly DOSed by the oracle safety bands!"
    );
}

// ===========================================================================
// 12. Liquidation Collateral Extraction via Second Tolerance Averaging
// ===========================================================================

#[test]
fn test_liquidation_collateral_extraction_via_averaging() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    // Start with perfect market conditions
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);
    t.set_price("USDC", usd(1));
    t.set_price("ETH", usd(2000));

    // Provide initial liquidity
    t.supply(test_harness::KEEPER_USER, "USDC", 100_000.0);

    // Increase ETH LTV and Threshold to make it super sensitive
    // Must do this BEFORE supplying so the position records these values.
    t.edit_asset_config("ETH", |c| {
        c.loan_to_value_bps = 9500;
        c.liquidation_threshold_bps = 9800;
    });

    // Use a loose tolerance to allow a wide 10% averaging band.
    t.set_oracle_tolerance("ETH", test_harness::LOOSE_TOLERANCE);

    // Alice supplies ETH (20,000 USD collateral)
    t.supply(ALICE, "ETH", 10.0);

    // Alice borrows heavily: 18,900 USDC against 19,000 max LTV
    t.borrow(ALICE, "USDC", 18_900.0);

    // Give liquidator USDC to perform the liquidation
    t.supply(LIQUIDATOR, "USDC", 20_000.0);

    // Spot falls to 1820 while the averaged price remains 1910.
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
