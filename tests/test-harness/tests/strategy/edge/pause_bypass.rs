//! Per-spoke pause must hold across the strategy wrappers, not just the direct
//! withdraw/repay paths. Strategy flows reuse `execute_withdrawal` /
//! `execute_repayment` / `execute_withdraw_all`, which previously skipped the
//! `enforce_spoke_asset_flags` guard the direct paths run inline. Liquidation
//! calls the lower-level `settle_*` helpers directly and must stay reachable so
//! keepers can still clear bad debt on a paused listing.

use super::*;
use test_harness::{liquidatable_usdc_eth, LIQUIDATOR};

// swap_collateral withdraws the source collateral through the strategy
// `execute_withdrawal` wrapper. Pausing that asset must block the swap even
// though the direct-withdraw sibling (`test_swap_collateral_no_borrows_skip_hf`)
// shows the same call succeeds when unpaused.
#[test]
fn test_swap_collateral_paused_collateral_reverts() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.fund_router("ETH", 5.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 5_0000000);

    t.set_spoke_asset_paused("USDC", true);

    // Direct and strategy paths must now agree: both reject the paused asset.
    assert_contract_error(
        t.try_withdraw(ALICE, "USDC", 100.0),
        errors::SPOKE_ASSET_PAUSED,
    );
    let result = t.try_swap_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps);
    assert_contract_error(result, errors::SPOKE_ASSET_PAUSED);
}

// repay_debt_with_collateral repays the target debt through the strategy
// `execute_repayment` wrapper. Pausing the debt asset must block the repay leg.
#[test]
fn test_repay_debt_with_collateral_paused_debt_reverts() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.fund_router("ETH", 1.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 1_0000000);

    t.set_spoke_asset_paused("ETH", true);

    // Direct and strategy repay paths must both reject the paused debt asset.
    assert_contract_error(t.try_repay(ALICE, "ETH", 0.5), errors::SPOKE_ASSET_PAUSED);
    let result = t.try_repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps, false);
    assert_contract_error(result, errors::SPOKE_ASSET_PAUSED);
}

// close_position drains every remaining collateral through the
// `execute_withdraw_all` wrapper. Here only the residual-drain asset (WBTC) is
// paused: the primary withdraw (USDC) and repay (ETH) legs pass, so the revert
// can only originate in the close-time `execute_withdraw_all` guard.
#[test]
fn test_close_position_paused_residual_collateral_reverts() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(ALICE, "WBTC", 0.1);
    t.borrow(ALICE, "ETH", 1.0);
    t.fund_router("ETH", 1.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 1_0000000);

    t.set_spoke_asset_paused("WBTC", true);

    let result = t.try_repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps, true);
    assert_contract_error(result, errors::SPOKE_ASSET_PAUSED);
}

// Regression guard for the fix's placement: the pause check lives in the
// strategy wrappers, NOT the shared `settle_*` helpers that liquidation reuses.
// Pausing both the seized collateral and the repaid debt must leave liquidation
// reachable, otherwise an incident pause would freeze bad debt in place.
#[test]
fn test_liquidation_of_paused_assets_still_succeeds() {
    let mut t = liquidatable_usdc_eth();

    t.set_spoke_asset_paused("USDC", true);
    t.set_spoke_asset_paused("ETH", true);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert!(
        result.is_ok(),
        "liquidation must stay reachable on paused assets: {result:?}"
    );
}
