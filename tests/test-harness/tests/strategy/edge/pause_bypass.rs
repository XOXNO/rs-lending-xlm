use super::*;
use test_harness::{liquidatable_usdc_eth, LIQUIDATOR};

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

    assert_contract_error(
        t.try_withdraw(ALICE, "USDC", 100.0),
        errors::SPOKE_ASSET_PAUSED,
    );
    let result = t.try_swap_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps);
    assert_contract_error(result, errors::SPOKE_ASSET_PAUSED);
}

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

    assert_contract_error(t.try_repay(ALICE, "ETH", 0.5), errors::SPOKE_ASSET_PAUSED);
    let result = t.try_repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps, false);
    assert_contract_error(result, errors::SPOKE_ASSET_PAUSED);
}

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

/// A paused collateral no longer blocks the seizure leg; `no_seize` is the flag that does.
///
/// Pause is a user-activity halt, and seizure is pro-rata across an account's whole collateral
/// set, so gating seizure on `paused` made one paused listing a protocol-wide liquidation halt.
/// See ADR-0008.
#[test]
fn test_liquidation_of_paused_collateral_is_allowed_but_no_seize_blocks_it() {
    let mut t = liquidatable_usdc_eth();

    t.set_spoke_asset_paused("USDC", true);
    assert!(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0).is_ok(),
        "pausing a collateral must not strand its holders"
    );

    t.set_spoke_asset_flags("USDC", true, false, true);
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::SPOKE_ASSET_SEIZURE_HALTED,
    );
}

#[test]
fn test_liquidation_of_paused_debt_reverts() {
    let mut t = liquidatable_usdc_eth();

    t.set_spoke_asset_paused("ETH", true);

    assert_contract_error(t.try_repay(ALICE, "ETH", 0.5), errors::SPOKE_ASSET_PAUSED);
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::SPOKE_ASSET_PAUSED,
    );
}
