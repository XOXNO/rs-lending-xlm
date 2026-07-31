use test_harness::{
    eth_preset, usd_cents, usdc_preset, usdt_stable_preset, LendingTest, ALICE, BOB, LIQUIDATOR,
    STABLECOIN_SPOKE,
};

#[test]
fn test_spoke_threshold_supersedes_asset_threshold() {
    let mut standard = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .build();

    standard.supply(ALICE, "USDC", 10_000.0);

    standard.borrow(ALICE, "USDT", 7_500.0);

    standard.set_price("USDC", usd_cents(93));
    standard.assert_liquidatable(ALICE);

    let mut spoke = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();
    spoke.create_spoke_account(ALICE, 2);
    spoke.supply(ALICE, "USDC", 10_000.0);
    spoke.borrow(ALICE, "USDT", 7_500.0);
    spoke.set_price("USDC", usd_cents(93));
    spoke.assert_healthy(ALICE);
}

#[test]
fn test_spoke_bonus_bounded_by_category_bonus() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .with_dust_disabled_all_markets()
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 9_500.0);

    t.set_price("USDC", usd_cents(85));
    t.assert_liquidatable(ALICE);

    t.get_or_create_user(LIQUIDATOR);
    let usdc_before = t.token_balance(LIQUIDATOR, "USDC");
    t.liquidate(LIQUIDATOR, ALICE, "USDT", 500.0);
    let usdc_after = t.token_balance(LIQUIDATOR, "USDC");

    let usdc_received = usdc_after - usdc_before;
    let usd_received = usdc_received * 0.85;
    let realized_bonus = (usd_received / 500.0) - 1.0;

    assert!(
        realized_bonus <= 0.03,
        "realized bonus in spoke must stay near 2 % (spoke cap), got {:.4}",
        realized_bonus
    );
    assert!(
        realized_bonus >= 0.005,
        "realized bonus should not be zero / negative, got {:.4}",
        realized_bonus
    );
}

#[test]
fn test_spoke_liquidation_with_split_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .with_dust_disabled_all_markets()
        .build();

    t.create_spoke_account(BOB, 2);
    t.supply(BOB, "USDT", 100_000.0);

    t.create_spoke_account(ALICE, 2);

    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "USDT", 4_000.0);
    t.borrow(ALICE, "USDT", 8_000.0);

    t.set_price("USDC", usd_cents(60));
    t.assert_liquidatable(ALICE);

    let usdc_collat_before = t.supply_balance(ALICE, "USDC");
    t.liquidate(LIQUIDATOR, ALICE, "USDT", 500.0);
    let usdc_collat_after = t.supply_balance(ALICE, "USDC");

    assert!(
        usdc_collat_after < usdc_collat_before,
        "USDC collateral must decrease after liquidation: before={:.4}, after={:.4}",
        usdc_collat_before,
        usdc_collat_after
    );
}

#[test]
fn test_spoke_rejects_non_category_collateral_addition() {
    use test_harness::{assert_contract_error, errors};

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 1_000.0);

    let result = t.try_supply(ALICE, "ETH", 0.1);
    assert_contract_error(result, errors::ASSET_NOT_IN_SPOKE);
}
