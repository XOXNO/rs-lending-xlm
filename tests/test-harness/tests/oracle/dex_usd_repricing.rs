//! Quote conversion on the read path, through explicit `Scaled` sources.
//!
//! v1 inferred the quote hop from a Reflector deployment's `base()`. The
//! composable model makes it explicit: a [`PriceSource::Scaled`] carries a ratio
//! feed and the [`PriceKey`] it is denominated in, and the engine resolves that
//! key recursively. Same repricing, but the dependency is written down — which
//! is what lets it be bounded (`MAX_RESOLUTION_DEPTH`), cycle-checked, and
//! prefetched.
//!
//! This is the SolvBTC shape: a ratio from one operator times a USD price from
//! another.

use controller::types::PriceKey;
use soroban_sdk::{Address, String, Vec};
use test_harness::{
    hub_asset, scaled_primary_redstone_anchor_config, scaled_single_config, usd, usd_frac,
    usdc_preset, xlm_preset, LendingTest, ALICE, DEFAULT_TOLERANCE,
};

/// A Reflector deployment publishing `asset` priced in some other unit. Its own
/// `base()` stays USD: the composable model does not read `base()` to infer a
/// hop, and a non-USD base is refused outright at configure time. What makes
/// this a *ratio* feed is how the config uses it — inside a `Scaled` source.
fn register_ratio_oracle(t: &LendingTest) -> Address {
    let dex = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    let client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
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

/// A `Scaled` source multiplies its ratio feed by the resolved price of its
/// quote key: XLM priced at 2.0 USDC, times USDC at $1.001, is $2.002.
#[test]
fn test_scaled_source_repriced_through_its_quote_key() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();

    // USDC trades just above peg so the multiply is observable in the result.
    t.set_price("USDC", usd_frac(1001, 1000)); // $1.001 / USDC

    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex = register_ratio_oracle(&t);
    let dex_client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex);
    dex_client.set_price(&xlm, &usd(2)); // XLM = 2.0 USDC on the DEX
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

    // 2.0 USDC * $1.001/USDC = $2.002
    assert_eq!(index_view(&t, &xlm).price_wad, usd_frac(2002, 1000));
}

/// The recursive resolution fits Soroban's default per-call budget on a
/// multi-asset HF path: a `Scaled` primary (one extra key resolution) plus a
/// RedStone USD anchor.
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

    let dex = register_ratio_oracle(&t);
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

    // Hot path under Soroban's default budget: the HF check prices XLM (which
    // recurses into USDC) and USDC. Completing == within budget.
    t.supply(ALICE, "XLM", 1_000.0);
    t.borrow(ALICE, "USDC", 100.0);
}

/// A quote key whose oracle is removed after listing takes the dependent market
/// down — closed, not silently repriced. The soft view reports it unusable
/// without reverting; the hard path reverts.
///
/// This is the dependency hazard the explicit model makes visible: validation
/// walks the graph as it is *now*, so a later edit to a dependency can only be
/// caught at read time.
#[test]
fn test_scaled_read_fails_closed_when_quote_key_loses_its_oracle() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    t.set_price("USDC", usd(1));
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex = register_ratio_oracle(&t);
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

    // The quote key stops being priceable.
    t.price_agg_client()
        .remove_oracle(&PriceKey::Token(usdc.clone()));

    // Soft view: the XLM row is unusable, never a revert.
    let row = index_view(&t, &xlm);
    assert!(!row.valid);
    assert_eq!(row.price_wad, 0);

    // Hard read path reverts rather than pricing without the quote.
    let mapped = match t.price_agg_client().try_price(&PriceKey::Token(xlm.clone())) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    test_harness::assert::assert_contract_error(
        mapped,
        test_harness::errors::OracleError::OracleNotConfigured as u32,
    );
}

/// Execute-time re-check: a config resolved while its quote key was healthy is
/// re-resolved when it is finally written, so an op that sat through the
/// timelock while the dependency broke cannot land.
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

    let dex = register_ratio_oracle(&t);
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

    // The op was resolved while USDC was healthy; the dependency breaks during
    // the delay.
    let stale = t
        .price_agg_client()
        .oracle(&PriceKey::Token(xlm.clone()))
        .unwrap();
    t.price_agg_client()
        .remove_oracle(&PriceKey::Token(usdc.clone()));

    // Writing the stale op re-resolves and refuses.
    t.price_agg_client()
        .set_oracle(&PriceKey::Token(xlm.clone()), &stale);
}

/// Happy path: replaying the same resolved config while the quote key is still
/// healthy applies unchanged.
#[test]
fn test_scaled_config_write_accepts_a_healthy_quote_key() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();
    t.set_price("USDC", usd(1));
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    let dex = register_ratio_oracle(&t);
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

/// Conversion happens per-source BEFORE the agreement band. A `Scaled` primary
/// and a USD anchor agree while the quote is pegged; a depeg moves the converted
/// primary away from the anchor and trips the band.
///
/// This is why the band cannot be applied to raw leg values: one leg is quoted
/// and the other is not, so comparing them before conversion would compare two
/// different units and pass on a real disagreement.
#[test]
fn test_scaled_primary_and_usd_anchor_tolerance_evaluated_after_conversion() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(xlm_preset())
        .build();

    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");

    // USDC pegged at $1.00.
    t.set_price("USDC", usd(1));

    // Ratio primary: XLM = 2.0 USDC (spot + twap).
    let dex = register_ratio_oracle(&t);
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
