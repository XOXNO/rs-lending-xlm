use controller::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, PriceKey, PriceSource,
    ProviderKind, ProviderRef, ScaledSource,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, String, Symbol, Vec};
use test_harness::{usd, usdc_preset, xlm_preset, LendingTest, DEFAULT_TOLERANCE};

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

fn quoted_reflector_config(t: &LendingTest, oracle: &Address, asset: &Address) -> AssetOracle {
    test_harness::reflector_primary_anchor_config(
        &t.env,
        oracle,
        asset,
        usd(2),
        DEFAULT_TOLERANCE.tolerance_bps,
    )
}

#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_quoted_reflector_rejected_as_direct_feed() {
    let t = LendingTest::new().with_market(xlm_preset()).build();
    let xlm = t.resolve_asset("XLM");

    let phantom_quote = Address::generate(&t.env);
    let dex = register_dex_oracle(&t, &phantom_quote);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex).set_price(&xlm, &usd(2));

    let cfg = quoted_reflector_config(&t, &dex, &xlm);
    t.configure_market_oracle(&xlm, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_quoted_reflector_rejected_even_when_quote_is_a_live_market() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex_usdc = register_dex_oracle(&t, &usdc);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_usdc)
        .set_price(&xlm, &usd(2));

    let cfg = quoted_reflector_config(&t, &dex_usdc, &xlm);
    t.configure_market_oracle(&xlm, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_quoted_reflector_rejected_when_quote_is_self() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    let xlm = t.resolve_asset("XLM");

    let dex = register_dex_oracle(&t, &xlm);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex).set_price(&xlm, &usd(2));

    let cfg = quoted_reflector_config(&t, &dex, &xlm);
    t.configure_market_oracle(&xlm, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #216)")]
fn test_scaled_source_rejected_when_quote_key_has_no_oracle() {
    let t = LendingTest::new().with_market(xlm_preset()).build();
    let xlm = t.resolve_asset("XLM");
    let adapter = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());

    let quote = PriceKey::Ref(Symbol::new(&t.env, "NOTHING"));
    let cfg = scaled_config(&t, &adapter, quote);
    t.configure_market_oracle(&xlm, &cfg);
}

#[test]
#[should_panic(expected = "Error(Contract, #225)")]
fn test_scaled_source_rejected_when_quote_key_is_self() {
    let t = LendingTest::new().with_market(xlm_preset()).build();
    let xlm = t.resolve_asset("XLM");
    let adapter = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());

    let cfg = scaled_config(&t, &adapter, PriceKey::Token(xlm.clone()));
    t.configure_market_oracle(&xlm, &cfg);
}

fn scaled_config(t: &LendingTest, adapter: &Address, quote: PriceKey) -> AssetOracle {
    let factor = FeedSource {
        provider: ProviderRef::MultiFeed(MultiFeedRef {
            contract: adapter.clone(),
            feed_id: String::from_str(&t.env, "RATIO"),
            kind: ProviderKind::RedStone,
            nature: FeedNature::Fundamental,
        }),
        decimals: 8,
        max_stale_seconds: 900,
    };
    AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        sources: Vec::from_array(
            &t.env,
            [PriceSource::Scaled(ScaledSource {
                factor,
                quote,
                min_factor_wad: 1,
                max_factor_wad: usd(1_000_000),
            })],
        ),
        tolerance: test_harness::tolerance_band(&t.env, DEFAULT_TOLERANCE.tolerance_bps),
        independence: IndependencePolicy::RequireDisjoint,

        min_sanity_price_wad: usd(2) - usd(2) / 100,
        max_sanity_price_wad: usd(2) + usd(2) / 100,
    }
}
