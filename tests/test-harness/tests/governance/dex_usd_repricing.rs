//! Quoted (non-USD-based) Reflector deployments, and the explicit quote path
//! that replaced them.
//!
//! v1 let a Reflector deployment quoted in a Stellar asset be used directly and
//! resolved the quote hop implicitly, so governance had to police which assets
//! were eligible quotes (configured market, itself USD-quoted, not self). The
//! composable model has no implicit hop: a direct `PriceSource::Feed` is read in
//! whatever unit the contract publishes, so a quoted deployment reaching one is
//! a config that would price the asset in the wrong unit. It is refused at
//! configure time (`InvalidOracleBase`, #220) by the provider attestation.
//!
//! Quoting is still expressible — as a `PriceSource::Scaled` naming its quote
//! key. The eligibility questions v1 answered with bespoke rules now fall out of
//! ordinary key resolution: an unconfigured quote key has no oracle, and a key
//! quoting itself is a cycle.

use controller::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, PriceKey, PriceSource,
    ProviderKind, ProviderRef, ScaledSource,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, String, Symbol, Vec};
use test_harness::{usd, usdc_preset, xlm_preset, LendingTest, DEFAULT_TOLERANCE};

/// Register a Reflector deployment quoted in `quote` (a Stellar SAC) rather
/// than in USD.
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

/// A smoothed single source on `oracle`, so the smoothing rule (#38) is
/// satisfied and the assertion lands on the check under test.
fn quoted_reflector_config(t: &LendingTest, oracle: &Address, asset: &Address) -> AssetOracle {
    test_harness::reflector_primary_anchor_config(
        &t.env,
        oracle,
        asset,
        usd(2),
        DEFAULT_TOLERANCE.tolerance_bps,
    )
}

/// A quoted Reflector deployment used as a direct feed is rejected regardless of
/// what it is quoted in: the price it returns is not denominated in USD, and a
/// direct feed carries no conversion.
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

/// Same refusal when the quote asset *is* a configured, USD-priced market. v1
/// accepted this shape; the composable model does not, because eligibility of
/// the quote is not what made an implicit hop safe — the absence of the hop is.
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

/// Self-quoting needs no bespoke rule now: it is refused by the same base check
/// as any other non-USD base.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_quoted_reflector_rejected_when_quote_is_self() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    let xlm = t.resolve_asset("XLM");

    let dex = register_dex_oracle(&t, &xlm); // base = the asset itself
    test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex).set_price(&xlm, &usd(2));

    let cfg = quoted_reflector_config(&t, &dex, &xlm);
    t.configure_market_oracle(&xlm, &cfg);
}

/// The explicit replacement: a `Scaled` source multiplies a ratio feed by a
/// named quote key. Naming a key with no stored oracle is refused at configure
/// time (`OracleNotConfigured`, #216) rather than reverting on first read.
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

/// A `Scaled` source naming its own key is a cycle. The key is pushed on the
/// resolution stack before its own sources are walked, so the self-edge is seen
/// while validating (`OracleCycleDetected`, #225) and can never be stored.
///
/// XLM is already configured by the preset, so the self-reference resolves to a
/// real stored oracle and reaches the cycle guard. Without a prior config the
/// same edit stops one step earlier, at `OracleNotConfigured`.
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

/// Single `Scaled` source: an 8-decimal ratio feed times `quote`. The ratio leg
/// is `Fundamental`, so one source satisfies the smoothing rule.
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
        // Tight: a single-source oracle's band is its only backstop, so a
        // full-domain band is refused (#226) before the quote is ever resolved.
        min_sanity_price_wad: usd(2) - usd(2) / 100,
        max_sanity_price_wad: usd(2) + usd(2) / 100,
    }
}
