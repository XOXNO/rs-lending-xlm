//! Quote-market eligibility for DEX-quoted Reflector sources, validated by
//! governance at configure time via the controller's market views.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_harness::{
    eth_preset, reflector_single_spot_config, usd, usdc_preset, xlm_preset, LendingTest,
    DEFAULT_TOLERANCE,
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

fn configure(t: &LendingTest, asset: &Address, cfg: &controller::types::AssetOracleConfigInput) {
    t.configure_market_oracle(asset, cfg);
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

    let cfg = reflector_single_spot_config(&dex, &xlm, usd(2), DEFAULT_TOLERANCE.tolerance_bps);
    configure(&t, &xlm, &cfg);
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
    let xlm_cfg =
        reflector_single_spot_config(&dex_usdc, &xlm, usd(2), DEFAULT_TOLERANCE.tolerance_bps);
    configure(&t, &xlm, &xlm_cfg);

    // Now try to quote ETH in XLM. XLM is Stellar-quoted, not USD → rejected.
    let dex_xlm = register_dex_oracle(&t, &xlm);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_xlm)
        .set_price(&eth, &usd(2));
    let eth_cfg =
        reflector_single_spot_config(&dex_xlm, &eth, usd(2), DEFAULT_TOLERANCE.tolerance_bps);
    configure(&t, &eth, &eth_cfg);
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

    let cfg = reflector_single_spot_config(&dex, &xlm, usd(2), DEFAULT_TOLERANCE.tolerance_bps);
    configure(&t, &xlm, &cfg);
}
