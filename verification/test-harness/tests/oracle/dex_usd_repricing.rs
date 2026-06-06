use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, String, Vec};
use test_harness::{
    eth_preset, reflector_primary_redstone_anchor_config, reflector_single_spot_config, usd,
    usd_frac, usdc_preset, xlm_preset, LendingTest, ALICE, DEFAULT_TOLERANCE,
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

fn index_view(t: &LendingTest, asset: &Address) -> common::types::MarketIndexView {
    let assets = Vec::from_array(&t.env, [asset.clone()]);
    t.ctrl_client()
        .get_all_market_indexes_detailed(&assets)
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
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.ctrl_client()
        .configure_market_oracle(&t.admin(), &xlm, &cfg);

    // 2.0 USDC * $1.001/USDC = $2.002
    assert_eq!(index_view(&t, &xlm).price_wad, usd_frac(2002, 1000));
}

/// Configuring a DEX-quoted source whose quote asset is not a configured market
/// is rejected (`InvalidOracleBase`, #220).
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_dex_config_rejected_when_quote_market_missing() {
    let t = LendingTest::new().with_market(xlm_preset()).build();
    let xlm = t.resolve_asset("XLM");

    // Base points at an asset with no configured market.
    let phantom_quote = Address::generate(&t.env);
    let dex = register_dex_oracle(&t, &phantom_quote);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex).set_price(&xlm, &usd(2));

    let cfg = reflector_single_spot_config(
        &dex,
        &xlm,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.ctrl_client()
        .configure_market_oracle(&t.admin(), &xlm, &cfg);
}

/// One-hop / cycle prevention: a quote asset that is itself DEX-quoted (not
/// USD-quoted) is rejected as a quote (`InvalidOracleBase`, #220). A
/// Stellar-quoted market can therefore never be the target of a quote edge.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_dex_config_rejected_when_quote_market_not_usd_quoted() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .with_market(eth_preset())
        .build();
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");
    let eth = t.resolve_asset("ETH");

    // Make XLM itself DEX-quoted (in USDC). Valid: USDC is USD-quoted.
    let dex_usdc = register_dex_oracle(&t, &usdc);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_usdc)
        .set_price(&xlm, &usd(2));
    let xlm_cfg = reflector_single_spot_config(
        &dex_usdc,
        &xlm,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.ctrl_client()
        .configure_market_oracle(&t.admin(), &xlm, &xlm_cfg);

    // Now try to quote ETH in XLM. XLM is Stellar-quoted, not USD → rejected.
    let dex_xlm = register_dex_oracle(&t, &xlm);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_xlm).set_price(&eth, &usd(2));
    let eth_cfg = reflector_single_spot_config(
        &dex_xlm,
        &eth,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.ctrl_client()
        .configure_market_oracle(&t.admin(), &eth, &eth_cfg);
}

/// A market may not be quoted in itself (`base == Stellar(self)`): rejected at
/// config time (`InvalidOracleBase`, #220) rather than reverting on recursion.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_dex_config_rejected_when_quote_is_self() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    let xlm = t.resolve_asset("XLM");

    let dex = register_dex_oracle(&t, &xlm); // base = the asset itself
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex).set_price(&xlm, &usd(2));

    let cfg = reflector_single_spot_config(
        &dex,
        &xlm,
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.ctrl_client()
        .configure_market_oracle(&t.admin(), &xlm, &cfg);
}

/// The DEX repricing path (the extra `base()` call plus the recursive
/// `token_price(quote)`) fits Soroban's default per-call budget on a real
/// multi-asset HF path. Guards the protocol-wide `base()` call now made on every
/// Reflector read. Uses the realistic mainnet shape: DEX (USDC-quoted) primary +
/// RedStone (USD) anchor.
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
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.ctrl_client()
        .configure_market_oracle(&t.admin(), &xlm, &cfg);

    // Hot path under Soroban's default budget: the HF check prices XLM (DEX→USD
    // recursion through token_price(USDC)) and USDC. Completing == within budget.
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
    t.ctrl_client().configure_market_oracle(
        &t.admin(),
        &xlm,
        &reflector_single_spot_config(
            &dex_usdc,
            &xlm,
            DEFAULT_TOLERANCE.first_upper_bps,
            DEFAULT_TOLERANCE.last_upper_bps,
        ),
    );

    // Reconfigure USDC itself to quote in ETH (another USD market): USDC is now
    // Stellar-quoted, so XLM->USDC would become a two-hop chain.
    let dex_eth = register_dex_oracle(&t, &eth);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_eth)
        .set_price(&usdc, &usd(1));
    t.ctrl_client().configure_market_oracle(
        &t.admin(),
        &usdc,
        &reflector_single_spot_config(
            &dex_eth,
            &usdc,
            DEFAULT_TOLERANCE.first_upper_bps,
            DEFAULT_TOLERANCE.last_upper_bps,
        ),
    );

    // Reading XLM must revert: USDC is no longer a direct USD market.
    index_view(&t, &xlm);
}

/// A Disabled quote market is rejected at read time regardless of the caller's
/// policy: a dependent market cannot be priced off a disabled quote.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_dex_read_rejects_disabled_quote_market() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    t.set_price("USDC", usd(1));
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex = register_dex_oracle(&t, &usdc);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex).set_price(&xlm, &usd(2));
    t.ctrl_client().configure_market_oracle(
        &t.admin(),
        &xlm,
        &reflector_single_spot_config(
            &dex,
            &xlm,
            DEFAULT_TOLERANCE.first_upper_bps,
            DEFAULT_TOLERANCE.last_upper_bps,
        ),
    );

    // Disable USDC, then read XLM -> must revert (cannot price off a disabled quote).
    t.ctrl_client().disable_token_oracle(&t.admin(), &usdc);
    index_view(&t, &xlm);
}

/// Conversion happens per-source BEFORE the tolerance band: a DEX (USDC-quoted)
/// primary and a RedStone (USD) anchor agree while USDC is pegged, but a USDC
/// depeg moves the converted primary away from the USD anchor and trips the
/// band. If conversion happened after composition, the band would compare
/// USDC-quoted vs USD and the depeg would be invisible.
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
        DEFAULT_TOLERANCE.first_upper_bps,
        DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.ctrl_client()
        .configure_market_oracle(&t.admin(), &xlm, &cfg);

    // Pegged: converted primary 2.0*1.0 = 2.0 USD == anchor 2.0 USD → in band.
    let healthy = index_view(&t, &xlm);
    assert_eq!(healthy.price_wad, usd(2));
    assert!(healthy.within_first_tolerance);
    assert!(healthy.within_second_tolerance);

    // Depeg USDC to $0.90: converted primary 2.0*0.9 = 1.8 USD vs anchor 2.0
    // USD = 10% gap, beyond both the 2% and 5% bands.
    t.set_price("USDC", usd_frac(90, 100));
    let depegged = index_view(&t, &xlm);
    assert!(!depegged.within_first_tolerance);
    assert!(!depegged.within_second_tolerance);
}
