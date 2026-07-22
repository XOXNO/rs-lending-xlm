use soroban_sdk::{Address, String, Vec};
use test_harness::{
    eth_preset, hub_asset, reflector_primary_redstone_anchor_config, reflector_single_spot_config,
    usd, usd_frac, usdc_preset, xlm_preset, LendingTest, ALICE, DEFAULT_TOLERANCE,
};

/// Register a DEX-style Reflector oracle quoted in `quote` (a Stellar SAC).
fn register_dex_oracle(t: &LendingTest, quote: &Address) -> Address {
    let dex = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    let client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
    client.set_base_stellar(quote);
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

/// A Reflector source whose `base()` is the USDC SAC (the Stellar-DEX oracle)
/// is repriced into USD by multiplying its token-per-USDC price by the USDC
/// market's own USD price.
#[test]
fn test_dex_quoted_source_repriced_to_usd() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();

    // USDC trades just above peg so the multiply is observable in the result.
    t.set_price("USDC", usd_frac(1001, 1000)); // $1.001 / USDC

    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex = register_dex_oracle(&t, &usdc);
    let dex_client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
    dex_client.set_price(&xlm, &usd(2)); // XLM = 2.0 USDC on the DEX

    let cfg = reflector_single_spot_config(
        &dex,
        &xlm,
        usd_frac(2002, 1000),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&xlm, &cfg);

    // 2.0 USDC * $1.001/USDC = $2.002
    assert_eq!(index_view(&t, &xlm).price_wad, usd_frac(2002, 1000));
}

/// DEX repricing path fits Soroban's default per-call budget on a multi-asset
/// HF path. Uses DEX (USDC-quoted) primary plus RedStone (USD) anchor.
#[test]
fn test_dex_quoted_market_priced_within_default_budget() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .with_budget_enabled()
        .build();
    t.set_price("USDC", usd(1));
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex = register_dex_oracle(&t, &usdc);
    let dex_client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
    dex_client.set_price(&xlm, &usd(2));
    dex_client.set_twap_price(&xlm, &usd(2));

    let feed_id = String::from_str(&t.env, "XLM");
    let redstone = t
        .env
        .register(test_harness::mock_redstone::MockRedStonePriceFeed, ());
    test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &redstone)
        .set_price(&feed_id, &usd(2));

    let cfg = reflector_primary_redstone_anchor_config(
        &dex,
        &xlm,
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&xlm, &cfg);

    // Hot path under Soroban's default budget: the HF check prices XLM (DEX→USD
    // recursion through resolve_usd_price(USDC)) and USDC. Completing == within budget.
    t.supply(ALICE, "XLM", 1_000.0);
    t.borrow(ALICE, "USDC", 100.0);
}

/// Read-time one-hop enforcement: if the quote market is reconfigured to a
/// non-USD base AFTER a dependent market was set up, reading the dependent
/// market reverts (#220) instead of silently chaining two hops.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_dex_read_rejects_quote_reconfigured_to_non_usd() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .with_market(eth_preset())
        .build();
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");
    let eth = t.resolve_asset("ETH");

    // XLM quotes in USDC (USDC is USD-quoted at this point).
    let dex_usdc = register_dex_oracle(&t, &usdc);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_usdc)
        .set_price(&xlm, &usd(2));
    t.configure_market_oracle(
        &xlm,
        &reflector_single_spot_config(&dex_usdc, &xlm, usd(2), DEFAULT_TOLERANCE.tolerance_bps),
    );

    // Reconfigure USDC itself to quote in ETH (another USD market): USDC is now
    // Stellar-quoted, so XLM->USDC would become a two-hop chain.
    let dex_eth = register_dex_oracle(&t, &eth);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_eth)
        .set_price(&usdc, &usd(1));
    t.configure_market_oracle(
        &usdc,
        &reflector_single_spot_config(&dex_eth, &usdc, usd(1), DEFAULT_TOLERANCE.tolerance_bps),
    );

    // Reading XLM must revert: USDC is not a direct USD market.
    index_view(&t, &xlm);
}

/// Execute-time re-check: if the quote market loses USD base during the
/// timelock delay, replaying the resolved config reverts (#220).
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_oracle_config_execute_rejects_quote_reconfigured_during_delay() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .with_market(eth_preset())
        .build();
    t.set_price("USDC", usd(1));
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");
    let eth = t.resolve_asset("ETH");

    // Configure XLM quoted in USDC while USDC is Active+USD (propose-time view).
    let dex = register_dex_oracle(&t, &usdc);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex).set_price(&xlm, &usd(2));
    t.configure_market_oracle(
        &xlm,
        &reflector_single_spot_config(&dex, &xlm, usd(2), DEFAULT_TOLERANCE.tolerance_bps),
    );

    // Capture the resolved config governance scheduled for the controller setter.
    let stale = t.price_agg_client().oracle_config(&xlm).unwrap();

    // During the delay, reconfigure USDC to quote in ETH (not a direct USD market).
    let dex_eth = register_dex_oracle(&t, &eth);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_eth)
        .set_price(&usdc, &usd(1));
    t.configure_market_oracle(
        &usdc,
        &reflector_single_spot_config(&dex_eth, &usdc, usd(1), DEFAULT_TOLERANCE.tolerance_bps),
    );

    // Executing the stale op re-asserts the quote invariant and reverts.
    t.price_agg_client().set_oracle_config(&xlm, &stale);
}

/// Happy path: re-applying the same resolved config while the quote market is
/// still Active+USD passes the execute-time re-check (no behavior change for
/// valid configs).
#[test]
fn test_oracle_config_execute_accepts_active_usd_quote_market() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    t.set_price("USDC", usd(1));
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex = register_dex_oracle(&t, &usdc);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex).set_price(&xlm, &usd(2));
    t.configure_market_oracle(
        &xlm,
        &reflector_single_spot_config(&dex, &xlm, usd(2), DEFAULT_TOLERANCE.tolerance_bps),
    );

    let resolved = t.price_agg_client().oracle_config(&xlm).unwrap();

    // USDC stays Active+USD: replaying the resolved config still applies.
    t.price_agg_client().set_oracle_config(&xlm, &resolved);
    assert_eq!(index_view(&t, &xlm).price_wad, usd(2));
}

/// Conversion happens per-source BEFORE the tolerance band: a DEX (USDC-quoted)
/// primary and a RedStone (USD) anchor agree while USDC is pegged, but a USDC
/// depeg moves the converted primary away from the USD anchor and trips the
/// band. Soft view reports `deviation`; write-path still reverts.
#[test]
fn test_dex_primary_redstone_anchor_tolerance_evaluated_in_usd() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();

    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    // USDC pegged at $1.00.
    t.set_price("USDC", usd(1));

    // DEX primary: XLM = 2.0 USDC (spot + twap).
    let dex = register_dex_oracle(&t, &usdc);
    let dex_client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
    dex_client.set_price(&xlm, &usd(2));
    dex_client.set_twap_price(&xlm, &usd(2));

    // RedStone anchor: XLM = 2.0 USD.
    let feed_id = String::from_str(&t.env, "XLM");
    let redstone = t
        .env
        .register(test_harness::mock_redstone::MockRedStonePriceFeed, ());
    test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &redstone)
        .set_price(&feed_id, &usd(2));

    let cfg = reflector_primary_redstone_anchor_config(
        &dex,
        &xlm,
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&xlm, &cfg);

    // Pegged: converted primary 2.0*1.0 = 2.0 USD == anchor 2.0 USD → in band,
    // blended to the midpoint $2.
    let ok = index_view(&t, &xlm);
    assert_eq!(ok.price_wad, usd(2));
    assert!(ok.valid);

    // Depeg USDC to $0.90: converted primary 2.0*0.9 = 1.8 USD vs anchor 2.0
    // USD = 10% gap → soft view marks deviation (write path still reverts).
    t.set_price("USDC", usd_frac(90, 100));
    let depegged = index_view(&t, &xlm);
    assert!(!depegged.valid);
    assert!(depegged.deviation);
}
