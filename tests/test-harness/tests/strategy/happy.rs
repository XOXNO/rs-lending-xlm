use controller::constants::WAD;
use soroban_sdk::Bytes;
use test_harness::{
    apply_flash_fee, build_aggregator_swap, eth_preset, hub_asset, usd, usdc_preset, wbtc_preset,
    LendingTest, MarketPreset, ALICE, BOB, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS,
    HARNESS_HUB, HARNESS_SPOKE,
};

fn usdc_zero_seed() -> MarketPreset {
    MarketPreset {
        name: "USDC",
        decimals: 7,
        price_wad: usd(1),
        initial_liquidity: 0.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

#[test]
fn test_multiply_creates_leveraged_position() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.fund_router("USDC", 3000.0);

    let steps = build_aggregator_swap(&t, "ETH", "USDC", apply_flash_fee(10_000_000), 3000_0000000);
    let account_id = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );

    assert!(account_id > 0, "account should be created");

    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (2999.0..=3001.0).contains(&supply),
        "USDC supply should be ~3000, got {}",
        supply
    );

    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&borrow),
        "ETH borrow should be ~1.0, got {}",
        borrow
    );

    let hf = t.health_factor_for(ALICE, account_id);
    assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
    assert!(hf < 2.0, "HF should be reasonable, got {}", hf);
}

#[test]
fn test_multiply_mode_long() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.fund_router("USDC", 3000.0);

    let steps = build_aggregator_swap(&t, "ETH", "USDC", apply_flash_fee(10_000_000), 3000_0000000);
    let account_id = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Long,
        &steps,
    );

    assert!(account_id > 0);

    let attrs = t.get_account_attributes(ALICE);
    assert_eq!(
        attrs.mode,
        controller::types::PositionMode::Long,
        "mode should be Long"
    );

    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (2999.0..=3001.0).contains(&supply),
        "USDC supply should be ~3000 in Long mode, got {}",
        supply
    );
    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&borrow),
        "ETH borrow should be ~1.0 in Long mode, got {}",
        borrow
    );

    let hf = t.health_factor_for(ALICE, account_id);
    assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
}

#[test]
fn test_multiply_mode_short() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.fund_router("USDC", 3000.0);

    let steps = build_aggregator_swap(&t, "ETH", "USDC", apply_flash_fee(10_000_000), 3000_0000000);
    let account_id = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Short,
        &steps,
    );

    assert!(account_id > 0);

    let attrs = t.get_account_attributes(ALICE);
    assert_eq!(
        attrs.mode,
        controller::types::PositionMode::Short,
        "mode should be Short"
    );

    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (2999.0..=3001.0).contains(&supply),
        "USDC supply should be ~3000 in Short mode, got {}",
        supply
    );
    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&borrow),
        "ETH borrow should be ~1.0 in Short mode, got {}",
        borrow
    );

    let hf = t.health_factor_for(ALICE, account_id);
    assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
}

#[test]
fn test_multiply_wbtc_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(wbtc_preset())
        .build();

    t.fund_router_raw("WBTC", 3_000_000);

    let steps = build_aggregator_swap(
        &t,
        "USDC",
        "WBTC",
        apply_flash_fee(10_000_000_000),
        3_000_000,
    );
    let account_id = t.multiply(
        ALICE,
        "WBTC",
        1000.0,
        "USDC",
        controller::types::PositionMode::Multiply,
        &steps,
    );

    assert!(account_id > 0);

    let supply = t.supply_balance_for(ALICE, account_id, "WBTC");
    assert!(supply > 0.0, "should have WBTC supply: got {}", supply);

    let borrow = t.borrow_balance_for(ALICE, account_id, "USDC");
    assert!(borrow > 0.0, "should have USDC borrow: got {}", borrow);

    let hf = t.health_factor_for(ALICE, account_id);
    assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
}

#[test]
fn test_swap_debt_replaces_borrow() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let initial_eth = t.borrow_balance(ALICE, "ETH");
    assert!(initial_eth > 0.9, "should have ~1 ETH borrow");

    t.fund_router("ETH", 1.0);

    let steps = build_aggregator_swap(&t, "WBTC", "ETH", apply_flash_fee(10_000_000), 1_0000000);
    t.swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);

    let wbtc_borrow = t.borrow_balance(ALICE, "WBTC");
    assert!(
        wbtc_borrow > 0.0,
        "should have WBTC borrow after swap: got {}",
        wbtc_borrow
    );

    let hf = t.health_factor(ALICE);
    assert!(hf >= 1.0, "HF should be >= 1.0 after swap_debt, got {}", hf);
}

#[test]
fn test_swap_debt_partial() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    t.supply(ALICE, "USDC", 200_000.0);
    t.borrow(ALICE, "ETH", 2.0);

    t.fund_router_raw("ETH", 5_000_000);

    let steps = build_aggregator_swap(&t, "WBTC", "ETH", apply_flash_fee(5_000_000), 5_000_000);
    t.swap_debt(ALICE, "ETH", 0.5, "WBTC", &steps);

    let eth_borrow = t.borrow_balance(ALICE, "ETH");
    assert!(
        eth_borrow > 0.0 && eth_borrow < 2.0,
        "ETH borrow should be partially repaid: got {}",
        eth_borrow
    );

    let wbtc_borrow = t.borrow_balance(ALICE, "WBTC");
    assert!(
        wbtc_borrow > 0.0,
        "should have WBTC borrow: got {}",
        wbtc_borrow
    );

    let hf = t.health_factor(ALICE);
    assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
}

#[test]
fn test_swap_collateral_replaces_supply() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let initial_usdc = t.supply_balance(ALICE, "USDC");
    assert!(initial_usdc >= 99_999.0, "should have ~100K USDC supply");

    t.fund_router("ETH", 10.0);

    let steps = build_aggregator_swap(&t, "USDC", "ETH", 200_000_000_000, 10_0000000);
    t.swap_collateral(ALICE, "USDC", 20_000.0, "ETH", &steps);

    let usdc_after = t.supply_balance(ALICE, "USDC");
    assert!(
        usdc_after < initial_usdc,
        "USDC supply should decrease: {} -> {}",
        initial_usdc,
        usdc_after
    );

    let eth_supply = t.supply_balance(ALICE, "ETH");
    assert!(
        (9.99..=10.01).contains(&eth_supply),
        "should have ~10 ETH supply: got {}",
        eth_supply
    );

    let hf = t.health_factor(ALICE);
    assert!(
        hf >= 1.0,
        "HF should be >= 1.0 after swap_collateral, got {}",
        hf
    );
}

#[test]
fn test_swap_collateral_no_borrows() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 50_000.0);

    t.fund_router("ETH", 5.0);

    let steps = build_aggregator_swap(&t, "USDC", "ETH", 100_000_000_000, 5_0000000);
    t.swap_collateral(ALICE, "USDC", 10_000.0, "ETH", &steps);

    let eth_supply = t.supply_balance(ALICE, "ETH");
    assert!(
        eth_supply > 0.0,
        "should have ETH supply: got {}",
        eth_supply
    );

    let usdc_supply = t.supply_balance(ALICE, "USDC");
    assert!(
        usdc_supply < 50_000.0,
        "USDC supply should be reduced: got {}",
        usdc_supply
    );
}

#[test]
fn test_repay_debt_with_collateral_reduces_positions() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let collateral_before = t.supply_balance(ALICE, "USDC");
    let debt_before = t.borrow_balance(ALICE, "ETH");

    t.fund_router("ETH", 0.5);

    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 5_000000);
    t.repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps, false);

    let collateral_after = t.supply_balance(ALICE, "USDC");
    let debt_after = t.borrow_balance(ALICE, "ETH");

    assert!(
        collateral_after < collateral_before,
        "USDC collateral should decrease: before={}, after={}",
        collateral_before,
        collateral_after
    );
    assert!(
        debt_after < debt_before,
        "ETH debt should decrease: before={}, after={}",
        debt_before,
        debt_after
    );
    assert!(
        t.health_factor(ALICE) >= 1.0,
        "HF should stay healthy after repay_debt_with_collateral"
    );
}

#[test]
fn test_repay_debt_with_collateral_same_token_succeeds_at_zero_cash() {
    let mut t = LendingTest::new()
        .with_market(usdc_zero_seed())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 20.0);
    t.borrow(ALICE, "USDC", 30_000.0);

    t.supply(BOB, "ETH", 1_000.0);
    t.borrow(BOB, "USDC", 63_000.0);

    let cash_before = t.pool_state_on_hub(HARNESS_HUB, "USDC").cash;
    let ten_thousand_usdc_raw = 100_000_000_000i128;
    assert!(
        cash_before < ten_thousand_usdc_raw,
        "precondition: market must hold less cash than the settle amount, got {cash_before}"
    );

    let debt_before = t.borrow_balance(ALICE, "USDC");
    let supply_before = t.supply_balance(ALICE, "USDC");

    let empty_steps = Bytes::new(&t.env);
    let result =
        t.try_repay_debt_with_collateral(ALICE, "USDC", 10_000.0, "USDC", &empty_steps, false);
    assert!(
        result.is_ok(),
        "same-hub same-asset repay must not need idle pool cash: {result:?}"
    );

    let cash_after = t.pool_state_on_hub(HARNESS_HUB, "USDC").cash;
    assert_eq!(
        cash_after, cash_before,
        "net-settle must not move cash at all"
    );

    let debt_after = t.borrow_balance(ALICE, "USDC");
    let supply_after = t.supply_balance(ALICE, "USDC");
    assert!(
        (debt_before - debt_after - 10_000.0).abs() < 100.0,
        "USDC debt should drop ~10k, actually dropped {}",
        debt_before - debt_after
    );
    assert!(
        (supply_before - supply_after - 10_000.0).abs() < 100.0,
        "USDC supply should drop ~10k, actually dropped {}",
        supply_before - supply_after
    );
}

#[test]
fn test_repay_debt_with_collateral_same_token_leaves_excess_as_supply() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 20.0);
    t.borrow(ALICE, "USDC", 10_000.0);

    let debt_before = t.borrow_balance(ALICE, "USDC");
    let supply_before = t.supply_balance(ALICE, "USDC");

    let empty_steps = Bytes::new(&t.env);
    t.repay_debt_with_collateral(ALICE, "USDC", 30_000.0, "USDC", &empty_steps, false);

    let debt_after = t.borrow_balance(ALICE, "USDC");
    let supply_after = t.supply_balance(ALICE, "USDC");

    assert!(
        debt_after < 1.0,
        "debt should be fully closed, got {debt_after}"
    );
    let supply_drop = supply_before - supply_after;
    assert!(
        (supply_drop - debt_before).abs() < 100.0,
        "supply should only drop by the debt actually owed (~{debt_before}), not the full 30k requested: dropped {supply_drop}"
    );
}

#[test]
fn test_same_token_net_settle_reduces_both_position_sides() {
    const UNIT: i128 = 10_000_000;

    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 20.0);
    t.borrow(ALICE, "USDC", 30_000.0);
    t.edit_asset_in_spoke_caps(
        "USDC",
        HARNESS_SPOKE,
        true,
        true,
        DEFAULT_ASSET_CONFIG.loan_to_value,
        DEFAULT_ASSET_CONFIG.liquidation_threshold,
        DEFAULT_ASSET_CONFIG.liquidation_bonus,
        120_000 * UNIT,
        50_000 * UNIT,
    );

    let account_id = t.resolve_account_id(ALICE);
    let usdc = hub_asset(t.resolve_asset("USDC"));
    let collateral_before = t.ctrl_client().get_collateral_amount(&account_id, &usdc);
    let debt_before = t.ctrl_client().get_borrow_amount(&account_id, &usdc);

    t.repay_debt_with_collateral(ALICE, "USDC", 10_000.0, "USDC", &Bytes::new(&t.env), false);

    let collateral_drop =
        collateral_before - t.ctrl_client().get_collateral_amount(&account_id, &usdc);
    let debt_drop = debt_before - t.ctrl_client().get_borrow_amount(&account_id, &usdc);
    assert!(
        (9_900 * UNIT..=10_000 * UNIT).contains(&collateral_drop),
        "net settlement should burn about 10,000 USDC of collateral, burned {collateral_drop}"
    );
    assert!(
        (9_900 * UNIT..=10_000 * UNIT).contains(&debt_drop),
        "net settlement should retire about 10,000 USDC of debt, retired {debt_drop}"
    );
}

#[test]
fn test_repay_debt_with_collateral_same_token_refreshes_risk_params() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 20.0);
    t.borrow(ALICE, "USDC", 10_000.0);
    let account_id = t.resolve_account_id(ALICE);

    let usdc = t.resolve_asset("USDC");
    let (supplies_before, _) = t.ctrl_client().get_account_positions(&account_id);
    let ltv_before = supplies_before
        .get(hub_asset(usdc.clone()))
        .expect("USDC position")
        .loan_to_value;

    let new_ltv = ltv_before - 500;
    t.edit_asset_in_spoke(
        "USDC",
        HARNESS_SPOKE,
        true,
        true,
        new_ltv,
        new_ltv + 300,
        200,
    );

    let empty_steps = Bytes::new(&t.env);
    t.repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "USDC", &empty_steps, false);

    let (supplies_after, _) = t.ctrl_client().get_account_positions(&account_id);
    let ltv_after = supplies_after
        .get(hub_asset(usdc))
        .expect("USDC position still open")
        .loan_to_value;
    assert_eq!(
        ltv_after, new_ltv,
        "net-settle must refresh risk params from current config, not keep the stale snapshot"
    );
}

#[test]
fn test_multiply_spoke_stablecoin() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    let caller = t.get_or_create_user(ALICE);
    let collateral_addr = t.resolve_asset("USDC");
    let debt_addr = t.resolve_asset("USDT");
    t.fund_router("USDC", 1050.0);

    let steps = build_aggregator_swap(
        &t,
        "USDT",
        "USDC",
        apply_flash_fee(10_000_000_000),
        1050_0000000,
    );

    let ctrl = t.ctrl_client();
    let account_id = ctrl.multiply(
        &caller,
        &0u64,
        &2u32,
        &hub_asset(collateral_addr.clone()),
        &1000_0000000i128,
        &hub_asset(debt_addr.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );

    assert!(account_id > 0, "spoke account should be created");

    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(supply > 0.0, "should have USDC supply in spoke");

    let borrow = t.borrow_balance_for(ALICE, account_id, "USDT");
    assert!(borrow > 0.0, "should have USDT borrow in spoke");

    let hf = ctrl.get_health_factor(&account_id);
    let hf_f64 = hf as f64 / (WAD as f64);
    assert!(hf_f64 >= 1.0, "spoke HF should be >= 1.0, got {}", hf_f64);
}

#[test]
fn test_multiply_large_amounts() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.fund_router("USDC", 300_000.0);

    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(1_000_000_000),
        3_000_000_000_000,
    );
    let account_id = t.multiply(
        ALICE,
        "USDC",
        100.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );

    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        supply >= 299_999.0,
        "should have ~300K USDC supply: got {}",
        supply
    );

    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        borrow >= 99.0,
        "should have ~100 ETH borrow: got {}",
        borrow
    );

    let hf = t.health_factor_for(ALICE, account_id);
    assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
}

#[test]
fn test_multiply_two_users() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply("liquidity_provider", "ETH", 10.0);

    t.fund_router("USDC", 3000.0);

    let steps_alice =
        build_aggregator_swap(&t, "ETH", "USDC", apply_flash_fee(10_000_000), 3000_0000000);
    let alice_id = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps_alice,
    );

    t.fund_router("USDC", 6000.0);

    let steps_bob =
        build_aggregator_swap(&t, "ETH", "USDC", apply_flash_fee(20_000_000), 6000_0000000);
    let bob_id = t.multiply(
        BOB,
        "USDC",
        2.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps_bob,
    );

    assert_ne!(alice_id, bob_id, "accounts should be different");

    let alice_supply = t.supply_balance_for(ALICE, alice_id, "USDC");
    let bob_supply = t.supply_balance_for(BOB, bob_id, "USDC");
    assert!(
        (2999.0..=3001.0).contains(&alice_supply),
        "Alice should have ~3000 USDC supply, got {}",
        alice_supply
    );
    assert!(
        (5999.0..=6001.0).contains(&bob_supply),
        "Bob should have ~6000 USDC supply, got {}",
        bob_supply
    );
    let alice_borrow = t.borrow_balance_for(ALICE, alice_id, "ETH");
    let bob_borrow = t.borrow_balance_for(BOB, bob_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&alice_borrow),
        "Alice should owe ~1 ETH, got {}",
        alice_borrow
    );
    assert!(
        (1.99..=2.01).contains(&bob_borrow),
        "Bob should owe ~2 ETH, got {}",
        bob_borrow
    );
    let alice_addr = t.get_or_create_user(ALICE);
    let bob_addr = t.get_or_create_user(BOB);
    assert_eq!(t.get_account_owner(alice_id), alice_addr);
    assert_eq!(t.get_account_owner(bob_id), bob_addr);

    let alice_hf = t.health_factor_for(ALICE, alice_id);
    let bob_hf = t.health_factor_for(BOB, bob_id);

    assert!(
        alice_hf >= 1.0,
        "Alice HF should be >= 1.0, got {}",
        alice_hf
    );
    assert!(bob_hf >= 1.0, "Bob HF should be >= 1.0, got {}", bob_hf);
}

#[test]
fn test_swap_debt_to_costlier_debt_preserves_minimum_hf() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    let hf_before = t.health_factor(ALICE);
    assert!(hf_before >= 1.0);

    t.fund_router("ETH", 10.0);

    let steps = build_aggregator_swap(&t, "WBTC", "ETH", apply_flash_fee(5_000_000), 10_0000000);
    t.swap_debt(ALICE, "ETH", 0.5, "WBTC", &steps);

    let hf_after = t.health_factor(ALICE);
    assert!(
        hf_after >= 1.0,
        "HF should still be >= 1.0 after swap_debt, got {}",
        hf_after
    );
    assert!(
        hf_after < hf_before,
        "HF must shrink when swapping to costlier debt: before={}, after={}",
        hf_before,
        hf_after
    );
}
