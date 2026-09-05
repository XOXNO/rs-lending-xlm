use controller::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, PriceKey, PriceSource,
    ProviderRef, ScaledSource,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, String, Symbol, Vec};
use test_harness::{hub_asset, usd, usdc_preset, xlm_preset, LendingTest, DEFAULT_TOLERANCE};

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
        provider: ProviderRef::RedStone(MultiFeedRef {
            contract: adapter.clone(),
            feed_id: String::from_str(&t.env, "RATIO"),
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

// --- Scaled factors: base must name exactly what the quote key prices -------
//
// `attest_sources` runs the same `attest_feed` on a `Scaled` factor as on a
// bare `Feed`, so wrapping in `Scaled` never bypassed the base check. What it
// now does instead is retarget it: a contract quoting in token X may only be
// scaled by `Token(X)`. These four cases pin the whole rule. Every other
// `Scaled` test in this file uses a RedStone factor, which has no base to
// check, which is why the Reflector-factor path went uncovered.

/// Registers a `Ref` oracle priced from a USD-quoting mock, so a `Ref` quote
/// key resolves during validation. There is no harness helper for non-`Token`
/// keys, so this drives the governance op directly.
fn register_ref_oracle(t: &LendingTest, key: &PriceKey, asset: &Address) {
    use governance::op::{AdminOperation, ConfigureAssetOracleArgs};

    let oracle_addr = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    let client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &oracle_addr);
    client.set_decimals(&14);
    client.set_resolution(&300);
    client.set_price(asset, &usd(1));

    let mut oracle = test_harness::reflector_primary_anchor_config(
        &t.env,
        &oracle_addr,
        asset,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    oracle.asset_decimals = 0; // PriceKey::Ref must declare 0

    t.gov_client().execute_immediate(
        &t.admin,
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: key.clone(),
            oracle,
        }),
    );
}

fn reflector_scaled_config(
    t: &LendingTest,
    oracle: &Address,
    asset: &Address,
    quote: PriceKey,
) -> AssetOracle {
    let factor = FeedSource {
        provider: ProviderRef::Reflector(controller::types::ReflectorFeedRef {
            contract: oracle.clone(),
            asset: controller::types::OracleAssetRef::Stellar(asset.clone()),
            read_mode: controller::types::OracleReadMode::Twap(3),
        }),
        decimals: 14,
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

// The mainnet AQUA/USTRY/CETES shape: the DEX oracle quotes in USDC and is
// scaled by Token(USDC), so the product is honestly USD-denominated.
#[test]
fn test_quoted_reflector_accepted_as_scaled_factor_matching_its_base() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex_usdc = register_dex_oracle(&t, &usdc);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_usdc)
        .set_price(&xlm, &usd(2));

    let cfg = reflector_scaled_config(&t, &dex_usdc, &xlm, PriceKey::Token(usdc.clone()));
    t.configure_market_oracle(&xlm, &cfg);

    // The point of the Scaled shape is the arithmetic, not the acceptance: a
    // DEX quote of 2 USDC per XLM times USDC at $1 must price XLM at $2. An
    // inverted composition (divide instead of multiply, or quote and factor
    // swapped) configures just as cleanly, so read the composed price back.
    let row = t
        .ctrl_client()
        .get_market_indexes_detailed(&Vec::from_array(&t.env, [hub_asset(xlm.clone())]))
        .get(0)
        .unwrap();
    assert!(row.valid, "an accepted Scaled config must price its asset");
    assert_eq!(
        row.price_wad,
        usd(2),
        "2 USDC per XLM x $1 per USDC = $2 per XLM"
    );
}

// The dangerous case the rule exists for: a factor denominated in one asset
// multiplied by the price of another. With two near-1.0 stablecoins the result
// is plausible and wrong, and no sanity band would catch it.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_scaled_factor_rejected_when_quote_is_not_the_factor_base() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    // Quote by a market that is registered and is not the key being configured,
    // so the config clears `validate_asset_oracle` and actually reaches attest:
    // the oracle quotes in XLM while the factor is scaled by USDC's price.
    let dex_xlm = register_dex_oracle(&t, &xlm);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_xlm)
        .set_price(&xlm, &usd(2));

    let cfg = reflector_scaled_config(&t, &dex_xlm, &xlm, PriceKey::Token(usdc.clone()));
    t.configure_market_oracle(&xlm, &cfg);
}

// A Ref names a synthetic reference with no on-chain asset identity, so the
// pairing cannot be proven either way.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_scaled_factor_rejected_when_quote_is_a_ref() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex_usdc = register_dex_oracle(&t, &usdc);
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_usdc)
        .set_price(&xlm, &usd(2));

    // Register the Ref first so the config clears `validate_asset_oracle`, which
    // would otherwise reject an unknown quote key with #216 before attest runs.
    // The Ref genuinely prices USDC here — attest still rejects it, because a
    // Ref carries no asset identity it could be checked against.
    let quote = PriceKey::Ref(Symbol::new(&t.env, "USDCQUOTE"));
    register_ref_oracle(&t, &quote, &usdc);

    let cfg = reflector_scaled_config(&t, &dex_usdc, &xlm, quote);
    t.configure_market_oracle(&xlm, &cfg);
}

// A USD-quoted contract needs no re-denomination; scaling it by a token price
// would double-count. No Token key prices USD, so this is unrepresentable.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_usd_quoted_reflector_rejected_as_scaled_factor() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let usd_oracle = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    let client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &usd_oracle);
    client.set_decimals(&14);
    client.set_resolution(&300);
    client.set_price(&xlm, &usd(2));

    let cfg = reflector_scaled_config(&t, &usd_oracle, &xlm, PriceKey::Token(usdc.clone()));
    t.configure_market_oracle(&xlm, &cfg);
}
