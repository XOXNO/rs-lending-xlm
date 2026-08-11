use super::{enable_dual_source, setup};
use test_harness::{assert_contract_error, errors, usd, usd_cents, LendingTest, ALICE, LIQUIDATOR};

#[test]
fn test_single_tolerance_uses_midpoint_price() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd_cents(103));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    t.assert_healthy(ALICE);
}

#[test]
fn test_exchange_source_safe_only() {
    let mut t = setup();
    t.set_oracle_primary_anchor("USDC");
    t.set_oracle_primary_anchor("ETH");

    t.set_safe_price("USDC", usd(1));
    t.set_safe_price("ETH", usd(2000));

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    t.assert_healthy(ALICE);
}

#[test]
fn test_mixed_tolerance_states() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd(1));

    t.set_safe_price("ETH", usd(2200));

    t.supply(ALICE, "USDC", 100_000.0);

    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}

#[test]
fn test_liquidation_blocked_under_flash_crash() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd(1));
    t.set_safe_price("ETH", usd(2000));
    t.set_price("USDC", usd(1));
    t.set_price("ETH", usd(2000));

    t.supply(test_harness::KEEPER_USER, "USDC", 100_000.0);

    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "USDC", 15_000.0);

    let hf_before = t.health_factor(ALICE);
    assert!(hf_before >= 1.0, "Alice should be healthy");

    t.set_price("ETH", usd(1400));
    t.set_safe_price("ETH", usd(1950));

    t.supply(LIQUIDATOR, "USDC", 20_000.0);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "USDC", 5_000.0);

    assert_contract_error(result, errors::UNSAFE_PRICE);
}

#[test]
fn test_liquidation_collateral_extraction_via_averaging() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.set_safe_price("USDC", usd(1));
    t.set_safe_price("ETH", usd(2000));
    t.set_price("USDC", usd(1));
    t.set_price("ETH", usd(2000));

    t.supply(test_harness::KEEPER_USER, "USDC", 100_000.0);

    t.edit_asset_config("ETH", |c| {
        c.loan_to_value = 9450;
        c.liquidation_threshold = 9500;
    });

    t.set_tolerance("ETH", test_harness::LOOSE_TOLERANCE);

    t.supply(ALICE, "ETH", 10.0);

    t.borrow(ALICE, "USDC", 18_175.0);

    t.supply(LIQUIDATOR, "USDC", 20_000.0);

    t.set_price("ETH", usd(1820));
    t.set_safe_price("ETH", usd(2000));

    let liquidator_eth_before = t.token_balance(LIQUIDATOR, "ETH");

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "USDC", 5_000.0);

    assert!(
        result.is_ok(),
        "Liquidation should succeed because 9% deviation is within loose 10% band!"
    );

    let liquidator_eth_after = t.token_balance(LIQUIDATOR, "ETH");
    let received_collateral = liquidator_eth_after - liquidator_eth_before;

    assert!(
        received_collateral > 2.7,
        "Liquidator successfully extracted excess collateral via averaging exploit: {}",
        received_collateral
    );

    // Fee-sensitive: what reaches the liquidator is the seizure net of the
    // protocol's cut, so this bound moves with the preset liquidation_fees.
    assert!(
        received_collateral > 2.73,
        "Liquidator successfully extracted excess collateral via averaging exploit: {}",
        received_collateral
    );
}

fn set_sanity_bounds(t: &LendingTest, asset_name: &str, min_wad: i128, max_wad: i128) {
    let asset = t.resolve_asset(asset_name);
    let mut oracle = t
        .price_agg_client()
        .oracle(&controller::types::PriceKey::Token(asset.clone()))
        .unwrap();
    oracle.min_sanity_price_wad = min_wad;
    oracle.max_sanity_price_wad = max_wad;
    t.price_agg_client()
        .seed_oracle(&controller::types::PriceKey::Token(asset.clone()), &oracle);
}

#[test]
fn test_sanity_bound_blocks_price_above_ceiling() {
    let mut t = setup();

    set_sanity_bounds(&t, "ETH", usd(100), usd(1_500));

    t.supply(ALICE, "USDC", 10_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::SANITY_BOUND_VIOLATED);
}

#[test]
fn test_sanity_bound_blocks_price_below_floor() {
    let mut t = setup();

    set_sanity_bounds(&t, "ETH", usd(3_000), usd(10_000));

    t.supply(ALICE, "USDC", 10_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::SANITY_BOUND_VIOLATED);
}

#[test]
fn test_sanity_bound_tampered_zero_state_rejected_at_runtime() {
    let mut t = setup();
    set_sanity_bounds(&t, "ETH", 0, 0);

    t.supply(ALICE, "USDC", 10_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::SANITY_BOUND_VIOLATED);
}
