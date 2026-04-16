extern crate std;

use soroban_sdk::token;
use test_harness::{eth_preset, usdc_preset, LendingTest, ALICE};

fn create_asset_contract(t: &LendingTest) -> soroban_sdk::Address {
    t.env
        .register_stellar_asset_contract_v2(t.admin())
        .address()
        .clone()
}

#[test]
fn test_disabled_market_blocks_supply_and_borrow() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let admin = t.admin();
    let eth_asset = t.resolve_asset("ETH");
    t.ctrl_client().disable_token_oracle(&admin, &eth_asset);

    t.supply(ALICE, "USDC", 10_000.0);

    let supply_result = t.try_supply(ALICE, "ETH", 1.0);
    assert!(
        supply_result.is_err(),
        "disabled market should block supply"
    );

    let borrow_result = t.try_borrow(ALICE, "ETH", 0.1);
    assert!(
        borrow_result.is_err(),
        "disabled market should block borrow"
    );
}

#[test]
fn test_disabled_debt_oracle_allows_repay_but_blocks_risk_increasing_ops() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let eth_asset = t.resolve_market("ETH").asset.clone();
    let admin = t.admin();
    t.ctrl_client().disable_token_oracle(&admin, &eth_asset);

    let repay_result = t.try_repay(ALICE, "ETH", 0.25);
    assert!(
        repay_result.is_ok(),
        "disabled debt oracle should still allow repay"
    );

    let borrow_result = t.try_borrow(ALICE, "ETH", 0.1);
    assert!(
        borrow_result.is_err(),
        "disabled debt oracle should block additional borrow"
    );

    let withdraw_result = t.try_withdraw(ALICE, "USDC", 1_000.0);
    assert!(
        withdraw_result.is_err(),
        "disabled debt oracle should block risk-increasing withdraw"
    );
}

#[test]
fn test_create_liquidity_pool_rejects_asset_id_mismatch() {
    let t = LendingTest::new().build();
    let ctrl = t.ctrl_client();

    let asset = create_asset_contract(&t);
    let wrong_asset = create_asset_contract(&t);
    let decimals = token::Client::new(&t.env, &asset).decimals();

    let params = usdc_preset()
        .params
        .to_market_params(&wrong_asset, decimals);
    let config = usdc_preset().config.to_asset_config();

    let result = match ctrl.try_create_liquidity_pool(&asset, &params, &config) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(err) => Err(err.expect("expected contract error")),
    };

    assert!(
        result.is_err(),
        "create_liquidity_pool should reject asset_id mismatch"
    );
}

#[test]
#[ignore]
fn test_create_liquidity_pool_rejects_asset_decimals_mismatch() {
    let t = LendingTest::new().build();
    let ctrl = t.ctrl_client();

    let asset = create_asset_contract(&t);
    let decimals = token::Client::new(&t.env, &asset).decimals();
    let mismatched_decimals = decimals.saturating_add(1);

    let params = usdc_preset()
        .params
        .to_market_params(&asset, mismatched_decimals);
    let config = usdc_preset().config.to_asset_config();

    let result = match ctrl.try_create_liquidity_pool(&asset, &params, &config) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(err) => Err(err.expect("expected contract error")),
    };

    assert!(
        result.is_err(),
        "create_liquidity_pool should reject asset_decimals mismatch"
    );
}
