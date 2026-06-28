use super::{enable_dual_source, setup};
use test_harness::{assert_contract_error, errors, usd, usd_cents, ALICE, LIQUIDATOR};

// 1. Price within first tolerance: all operations succeed

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
// 2. Price within second tolerance: operations still succeed

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
// 3. Price beyond second tolerance: risk-increasing ops blocked

#[test]
fn test_unsafe_price_allows_supply() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");

    // Set USDC safe price 10% from aggregator (beyond second tolerance of 5%).
    // Aggregator: $1.00, Safe: $1.10 (10% deviation).
    t.set_safe_price("USDC", usd_cents(110), true, true);

    // Supply still succeeds under the risk-decreasing oracle policy. Use the
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

    // Repay still succeeds under the permissive repay policy. Snapshot debt
    // before and after to verify the scaled borrow decreases.
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

    // Borrow fails: USDC (collateral) price is unsafe, and borrow uses the
    // strict risk-increasing policy.
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

    // Withdraw fails when the user has borrows because it uses the strict
    // risk-increasing policy.
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

    // With no debt, the withdraw cache uses the risk-decreasing policy,
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

    // With borrows present the cache uses the strict risk-increasing policy;
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
    // Liquidation hard-blocks when the primary and anchor sources diverge
    // beyond the tolerance band: the fail-closed price read rejects with
    // `OracleError::UnsafePriceNotAllowed` rather than seizing collateral at a
    // price only the spot source corroborates.
    //
    // Deliberate manipulation-over-availability tradeoff (auditors: this
    // reverses the §4.5 deviation-tolerance posture). The two price sources are
    // independent, so an attacker cannot hold them apart beyond both tolerance
    // bands for long — sustained out-of-band divergence is either a real
    // extreme event or requires manipulating both feeds at once.
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

    // Top up the liquidator so the seize path can actually pay.
    t.supply(LIQUIDATOR, "ETH", 5.0);

    // Deviate the USDC safe price beyond tolerance so the primary and anchor
    // sources sit outside the last band; the liquidation price read rejects.
    t.set_safe_price("USDC", usd_cents(110), true, true);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}
