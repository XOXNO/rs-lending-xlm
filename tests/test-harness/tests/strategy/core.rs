use controller::types::SpokeAssetArgs;
use test_harness::{
    assert_contract_error, build_aggregator_swap, errors, LendingTest, HARNESS_HUB, HARNESS_SPOKE,
};

use crate::helpers::AliceOps;

fn freeze_asset(t: &LendingTest, asset_name: &str) {
    let asset = t.resolve_asset(asset_name);
    let config = t.get_asset_config(asset_name);
    t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset,
        spoke_id: HARNESS_SPOKE,
        can_collateral: config.is_collateralizable,
        can_borrow: config.is_borrowable,
        paused: false,
        frozen: true,
        no_seize: false,
        ltv: config.loan_to_value,
        threshold: config.liquidation_threshold,
        bonus: config.liquidation_bonus,
        liquidation_fees: config.liquidation_fees,
        supply_cap: config.supply_cap,
        borrow_cap: config.borrow_cap,
    });
}

#[test]
fn test_multiply_rejects_non_borrowable_debt() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market_config("ETH", |c| {
            c.is_borrowable = false;
        })
        .build();

    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 1_0000000);
    let result = t.try_alice_multiply(&steps);
    assert_contract_error(result, errors::ASSET_NOT_BORROWABLE);
}

#[test]
fn test_multiply_rejects_non_collateralizable() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market_config("USDC", |c| {
            c.is_collateralizable = false;
        })
        .build();

    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 1000_0000000);
    let result = t.try_alice_multiply(&steps);
    assert_contract_error(result, errors::NOT_COLLATERAL);
}

#[test]
fn test_multiply_rejects_frozen_collateral() {
    let mut t = LendingTest::new().standard_two_asset().build();

    freeze_asset(&t, "USDC");

    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 1000_0000000);
    let result = t.try_alice_multiply(&steps);
    assert_contract_error(result, errors::SPOKE_ASSET_FROZEN);
}

#[test]
fn test_multiply_rejects_during_flash_loan() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.set_flash_loan_ongoing(true);

    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 1000_0000000);
    let result = t.try_alice_multiply(&steps);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}
