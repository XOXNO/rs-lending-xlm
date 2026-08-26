use controller::types::PriceKey;
use soroban_sdk::{Address, String, Vec};
use test_harness::{
    hub_asset, scaled_primary_redstone_anchor_config, scaled_single_config, usd, usd_frac,
    usdc_preset, xlm_preset, LendingTest, ALICE, DEFAULT_TOLERANCE,
};

/// A DEX oracle quotes in the pool's counter asset rather than in USD — that
/// is the whole reason these prices go through a `Scaled` source, whose quote
/// key re-denominates them. `attest` requires the contract's base and that
/// quote key to name the same asset, so the mock has to carry a real base.
fn register_ratio_oracle(t: &LendingTest, quote_base: &Address) -> Address {
    let dex = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    let client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
    client.set_base_stellar(quote_base);
    client.set_decimals(&14);
    client.set_resolution(&300);
    dex
}

fn index_view(t: &LendingTest, asset: &Address) -> controller::types::MarketIndexView {
    let assets = Vec::from_array(&t.env, [hub_asset(asset.clone())]);
    t.ctrl_client()
        .get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap()
}

#[test]
fn test_scaled_source_repriced_through_its_quote_key() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();

    t.set_price("USDC", usd_frac(1001, 1000));

    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex = register_ratio_oracle(&t, &usdc);
    let dex_client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
    dex_client.set_price(&xlm, &usd(2));
    dex_client.set_twap_price(&xlm, &usd(2));

    let cfg = scaled_single_config(
        &t.env,
        &dex,
        &xlm,
        PriceKey::Token(usdc.clone()),
        usd_frac(2002, 1000),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&xlm, &cfg);

    assert_eq!(index_view(&t, &xlm).price_wad, usd_frac(2002, 1000));
}

#[test]
fn test_scaled_market_priced_within_default_budget() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .with_budget_enabled()
        .build();
    t.set_price("USDC", usd(1));
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex = register_ratio_oracle(&t, &usdc);
    let dex_client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
    dex_client.set_price(&xlm, &usd(2));
    dex_client.set_twap_price(&xlm, &usd(2));

    let feed_id = String::from_str(&t.env, "XLM");
    let redstone = t
        .env
        .register(test_harness::mock_redstone::MockRedStonePriceFeed, ());
    test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &redstone)
        .set_price(&feed_id, &usd(2));

    let cfg = scaled_primary_redstone_anchor_config(
        &t.env,
        &dex,
        &xlm,
        PriceKey::Token(usdc.clone()),
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&xlm, &cfg);

    t.supply(ALICE, "XLM", 1_000.0);
    t.borrow(ALICE, "USDC", 100.0);
}

#[test]
fn test_scaled_read_fails_closed_when_quote_key_loses_its_oracle() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    t.set_price("USDC", usd(1));
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex = register_ratio_oracle(&t, &usdc);
    let dex_client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
    dex_client.set_price(&xlm, &usd(2));
    dex_client.set_twap_price(&xlm, &usd(2));
    t.configure_market_oracle(
        &xlm,
        &scaled_single_config(
            &t.env,
            &dex,
            &xlm,
            PriceKey::Token(usdc.clone()),
            usd(2),
            DEFAULT_TOLERANCE.tolerance_bps,
        ),
    );
    assert_eq!(index_view(&t, &xlm).price_wad, usd(2));

    t.price_agg_client()
        .remove_oracle(&PriceKey::Token(usdc.clone()));

    let row = index_view(&t, &xlm);
    assert!(!row.valid);
    assert_eq!(row.price_wad, 0);

    let mapped = match t
        .price_agg_client()
        .try_prices(&soroban_sdk::vec![&t.env, PriceKey::Token(xlm.clone())])
    {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    test_harness::assert::assert_contract_error(
        mapped,
        test_harness::errors::OracleError::OracleNotConfigured as u32,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #216)")]
fn test_scaled_config_write_rejects_quote_key_broken_during_delay() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    t.set_price("USDC", usd(1));
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex = register_ratio_oracle(&t, &usdc);
    let dex_client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
    dex_client.set_price(&xlm, &usd(2));
    dex_client.set_twap_price(&xlm, &usd(2));
    let cfg = scaled_single_config(
        &t.env,
        &dex,
        &xlm,
        PriceKey::Token(usdc.clone()),
        usd(2),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&xlm, &cfg);

    let stale = t
        .price_agg_client()
        .oracle(&PriceKey::Token(xlm.clone()))
        .unwrap();
    t.price_agg_client()
        .remove_oracle(&PriceKey::Token(usdc.clone()));

    t.price_agg_client()
        .set_oracle(&PriceKey::Token(xlm.clone()), &stale);
}

#[test]
fn test_scaled_config_write_accepts_a_healthy_quote_key() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    t.set_price("USDC", usd(1));
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex = register_ratio_oracle(&t, &usdc);
    let dex_client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
    dex_client.set_price(&xlm, &usd(2));
    dex_client.set_twap_price(&xlm, &usd(2));
    t.configure_market_oracle(
        &xlm,
        &scaled_single_config(
            &t.env,
            &dex,
            &xlm,
            PriceKey::Token(usdc.clone()),
            usd(2),
            DEFAULT_TOLERANCE.tolerance_bps,
        ),
    );

    let resolved = t
        .price_agg_client()
        .oracle(&PriceKey::Token(xlm.clone()))
        .unwrap();

    t.price_agg_client()
        .set_oracle(&PriceKey::Token(xlm.clone()), &resolved);
    assert_eq!(index_view(&t, &xlm).price_wad, usd(2));
}

#[test]
fn test_scaled_primary_and_usd_anchor_tolerance_evaluated_after_conversion() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();

    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    t.set_price("USDC", usd(1));

    let dex = register_ratio_oracle(&t, &usdc);
    let dex_client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
    dex_client.set_price(&xlm, &usd(2));
    dex_client.set_twap_price(&xlm, &usd(2));

    let feed_id = String::from_str(&t.env, "XLM");
    let redstone = t
        .env
        .register(test_harness::mock_redstone::MockRedStonePriceFeed, ());
    test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &redstone)
        .set_price(&feed_id, &usd(2));

    let cfg = scaled_primary_redstone_anchor_config(
        &t.env,
        &dex,
        &xlm,
        PriceKey::Token(usdc.clone()),
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&xlm, &cfg);

    let ok = index_view(&t, &xlm);
    assert_eq!(ok.price_wad, usd(2));
    assert!(ok.valid);

    t.set_price("USDC", usd_frac(90, 100));
    let depegged = index_view(&t, &xlm);
    assert!(!depegged.valid);
    assert!(depegged.deviation);
}
