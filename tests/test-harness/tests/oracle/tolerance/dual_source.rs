use super::{enable_dual_source, setup};
use test_harness::{assert_contract_error, errors, usd, usd_cents, LendingTest, ALICE, LIQUIDATOR};

// Dual-source: average price used in second tolerance zone.

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
// Exchange source = 1 (safe price only).

#[test]
fn test_exchange_source_safe_only() {
    let mut t = setup();
    t.set_oracle_primary_anchor("USDC");
    t.set_oracle_primary_anchor("ETH");

    // Set safe prices (used because exchange_source=1).
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_safe_price("ETH", usd(2000), true, true);

    // Operations succeed using the safe price alone.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    t.assert_healthy(ALICE);
}
// Multiple assets with different tolerance states.

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
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

#[test]
fn test_liquidation_blocked_under_flash_crash() {
    // Spot vs anchor beyond tolerance → fail-closed UnsafePriceNotAllowed (no seize on spot alone).
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
    // The flash crash
    // Spot ETH crashes to $1400 (a 30% drop). The anchor (TWAP) is slow and
    // still reads $1950. The deviation exceeds the second tolerance.
    t.set_price("ETH", usd(1400));
    t.set_safe_price("ETH", usd(1950), true, true);

    // Give the liquidator some USDC to perform the liquidation.
    t.supply(LIQUIDATOR, "USDC", 20_000.0);

    // The liquidator attempts a partial liquidation while spot and anchor sit
    // beyond the second tolerance band.
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "USDC", 5_000.0);

    // The out-of-band deviation is rejected: the protocol will not liquidate
    // against a price only the spot source corroborates. Liquidation resumes
    // once the anchor catches up and the sources reconverge within tolerance.
    assert_contract_error(result, errors::UNSAFE_PRICE);
}
// Liquidation collateral extraction via second-tolerance averaging.

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
        c.loan_to_value = 9450;
        c.liquidation_threshold = 9500;
    });

    // Use a loose tolerance to allow a wide 10% averaging band.
    t.set_oracle_tolerance("ETH", test_harness::LOOSE_TOLERANCE);

    // Alice supplies ETH (20,000 USD collateral).
    t.supply(ALICE, "ETH", 10.0);

    // Alice borrows heavily: 18,175 USDC against 19,000 max LTV. The margin
    // keeps blended C/D at ~1.051, just above 1 + base bonus, so a partial
    // liquidation stays HF-safe (the FullCloseRequired gate does not bind).
    t.borrow(ALICE, "USDC", 18_175.0);

    // Give the liquidator USDC to perform the liquidation.
    t.supply(LIQUIDATOR, "USDC", 20_000.0);

    // Spot falls to 1820 while the averaged price stays at 1910.
    // Threshold value = 10 * 1910 * 0.95 = 18,145, below the 18,175 debt.

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
// Sanity-bound circuit breaker.
//
// Slender M-7 / STELLAR_AUDIT_FINDINGS.md §4.2: a per-market absolute
// floor/ceiling must reject obviously-wrong oracle outputs (whether from a
// genuine feed bug or a brief spot manipulation under the `Liquidation`
// policy). Sentinel `max_sanity_price_wad == 0` keeps the check disabled for
// the rest of the test corpus; this test opts in by writing tight bounds
// directly to storage.

fn set_sanity_bounds(t: &LendingTest, asset_name: &str, min_wad: i128, max_wad: i128) {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller, || {
        let key = controller::types::ControllerKey::AssetOracle(asset.clone());
        let mut oracle: controller::types::MarketOracleConfig =
            t.env.storage().persistent().get(&key).unwrap();
        oracle.min_sanity_price_wad = min_wad;
        oracle.max_sanity_price_wad = max_wad;
        t.env.storage().persistent().set(&key, &oracle);
    });
}

#[test]
fn test_sanity_bound_blocks_price_above_ceiling() {
    let mut t = setup();
    // Default ETH price is $2,000. Cap at $1,500 → reads must revert.
    set_sanity_bounds(&t, "ETH", usd(100), usd(1_500));

    t.supply(ALICE, "USDC", 10_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::SANITY_BOUND_VIOLATED);
}

#[test]
fn test_sanity_bound_blocks_price_below_floor() {
    let mut t = setup();
    // Default ETH price is $2,000. Floor at $3,000 → reads must revert.
    set_sanity_bounds(&t, "ETH", usd(3_000), usd(10_000));

    t.supply(ALICE, "USDC", 10_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::SANITY_BOUND_VIOLATED);
}

// Runtime treats max_sanity==0 as a violation (storage-tamper defense).
#[test]
fn test_sanity_bound_tampered_zero_state_rejected_at_runtime() {
    let mut t = setup();
    set_sanity_bounds(&t, "ETH", 0, 0);

    t.supply(ALICE, "USDC", 10_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::SANITY_BOUND_VIOLATED);
}
