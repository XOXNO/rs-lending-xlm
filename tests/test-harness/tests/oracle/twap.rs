use controller::types::{ControllerKey, MarketOracleConfig, OracleReadMode, OracleSourceConfig};
use soroban_sdk::vec;
use test_harness::{
    assert_contract_error, errors, hub_asset, usd, usd_cents, usdc_preset, LendingTest, ALICE,
};

fn setup() -> LendingTest {
    LendingTest::new().dual_source_two_asset()
}

#[test]
fn configure_accepts_minimum_resolution_equal_to_max_stale() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let usdc = t.resolve_asset("USDC");
    t.mock_reflector_client().set_resolution(&60);
    let mut cfg = test_harness::reflector_single_spot_config(
        &t.mock_reflector,
        &usdc,
        usd(1),
        test_harness::DEFAULT_TOLERANCE.tolerance_bps,
    );
    cfg.max_price_stale_seconds = 60;

    t.configure_market_oracle(&usdc, &cfg);

    let stored: MarketOracleConfig = t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .get(&ControllerKey::AssetOracle(usdc))
            .expect("configured oracle")
    });
    assert_eq!(stored.max_price_stale_seconds, 60);
}

#[test]
#[should_panic(expected = "Error(Contract, #217)")]
fn configure_rejects_nonpositive_live_reflector_price() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let usdc = t.resolve_asset("USDC");
    t.mock_reflector_client().set_price(&usdc, &0);
    let cfg = test_harness::reflector_single_spot_config(
        &t.mock_reflector,
        &usdc,
        usd(1),
        test_harness::DEFAULT_TOLERANCE.tolerance_bps,
    );

    t.configure_market_oracle(&usdc, &cfg);
}

// `prices()` returns an empty Vec — drives `history.is_empty()` branch.
#[test]
fn test_empty_twap_history_blocks_strict_borrow() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &2);

    t.supply(ALICE, "USDC", 100_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::REFLECTOR_HISTORY_EMPTY);
}

// `prices()` returns fewer records than `min_twap_observations(records)` —
// drives the `history.len() < min_twap_observations(...)` branch.
#[test]
fn test_insufficient_twap_history_blocks_strict_borrow() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &3);

    t.supply(ALICE, "USDC", 100_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::TWAP_INSUFFICIENT_OBSERVATIONS);
}

#[test]
fn test_exact_minimum_twap_history_is_accepted() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &6);

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    assert!(t.health_factor(ALICE) > 1.0);
}

// Round-trip the `set_price` / `set_safe_price` plumbing so the
// `to_reflector_asset` / `read_spot` / observation-from-pricedata helpers
// see live data. `usd_cents` import is the only way to exercise the
// rounding path inside the spot reader for non-round numbers.
#[test]
fn test_spot_with_cents_price_supplies_cleanly() {
    let mut t = setup();
    t.set_price("USDC", usd_cents(99));
    t.supply(ALICE, "USDC", 5_000.0);
    t.assert_supply_near(ALICE, "USDC", 5_000.0, 1.0);
}

// Mode 4: one entry in the TWAP window has a non-positive price → the
// reader's `pd.price <= 0` branch fires and `has_invalid_price` flips,
// routing through the `InvalidPrice`-tagged `twap_fallback_or_panic`.
// Under strict policy this panics with `InvalidPrice`.
#[test]
fn test_twap_invalid_price_blocks_strict_borrow() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &4);

    t.supply(ALICE, "USDC", 100_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::INVALID_PRICE);
}

// Mode 5: oldest TWAP timestamp is far in the past → the staleness check
// against `oldest_ts` rejects under strict policy.
#[test]
fn test_twap_stale_history_blocks_strict_borrow() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &5);

    t.supply(ALICE, "USDC", 100_000.0);
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::PRICE_FEED_STALE);
}

// Fail-closed: there is no degraded fallback and no `twap_degraded` event.
// Reading market indexes after marking USDC's TWAP history empty reverts
// `ReflectorHistoryEmpty` (#212) even on the view path.
#[test]
#[should_panic(expected = "Error(Contract, #212)")]
fn test_twap_degradation_on_view_reverts() {
    let t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &2);

    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(usdc_asset)]);
    let _ = t.ctrl_client().get_market_indexes_detailed(&assets);
}

// `OracleReadMode::Spot` primary + missing `lastprice` → `read_spot` panics
// with `NoLastPrice` when `required=true`. Reconfigures the ETH market to
// `Single + Spot` so the read goes straight through `spot::read_spot`,
// then wipes the mock's spot entry. The borrow path is strict
// (`RiskIncreasing`) so `required=true`.
#[test]
#[should_panic(expected = "Error(Contract, #210)")]
fn test_reflector_spot_missing_lastprice_panics_under_strict() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    let eth_asset = t.resolve_asset("ETH");

    // Switch ETH to Single+Spot so the price path is `read_spot` only.
    let spot_cfg = test_harness::reflector_single_spot_config(
        &t.mock_reflector,
        &eth_asset,
        usd(2_000), // dual_source_two_asset's ETH default (WAD * 2_000).
        test_harness::DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&eth_asset, &spot_cfg);

    // Establish USDC collateral so the borrow path can reach the ETH price.
    let _ = usdc_asset;
    t.supply(ALICE, "USDC", 100_000.0);

    // Wipe the ETH spot from the mock reflector's temporary storage so the
    // next `lastprice` for ETH returns None.
    let reflector_addr = t.mock_reflector.clone();
    let eth_clone = eth_asset.clone();
    t.env.as_contract(&reflector_addr, || {
        let key = test_harness::mock_reflector::MockKey::Spot(eth_clone);
        t.env.storage().temporary().remove(&key);
    });

    // Borrow ETH — strict (RiskIncreasing) policy → `required=true`.
    t.borrow(ALICE, "ETH", 1.0);
}

// Fail-closed: `read_twap` with `records == 0` reverts
// `TwapInsufficientObservations` (#219); there is no anchor-spot fallback.
#[test]
#[should_panic(expected = "Error(Contract, #219)")]
fn test_twap_zero_records_reverts_on_view() {
    let t = LendingTest::new().dual_source_two_asset();
    let usdc = t.resolve_asset("USDC");
    t.env.as_contract(&t.controller, || {
        let key = ControllerKey::AssetOracle(usdc.clone());
        let mut oracle: MarketOracleConfig = t.env.storage().persistent().get(&key).unwrap();
        if let OracleSourceConfig::Reflector(ref mut source) = oracle.primary {
            source.read_mode = OracleReadMode::Twap(0);
        }
        t.env.storage().persistent().set(&key, &oracle);
    });

    let assets = vec![&t.env, hub_asset(usdc)];
    let _ = t.ctrl_client().get_market_indexes_detailed(&assets);
}

// TWAP requests above the protocol record cap are rejected.
#[test]
#[should_panic(expected = "Error(Contract, #204)")]
fn test_twap_records_above_max_rejects_on_view() {
    let t = LendingTest::new().dual_source_two_asset();
    let usdc = t.resolve_asset("USDC");
    t.env.as_contract(&t.controller, || {
        let key = ControllerKey::AssetOracle(usdc.clone());
        let mut oracle: MarketOracleConfig = t.env.storage().persistent().get(&key).unwrap();
        if let OracleSourceConfig::Reflector(ref mut source) = oracle.primary {
            source.read_mode = OracleReadMode::Twap(13);
        }
        t.env.storage().persistent().set(&key, &oracle);
    });

    let assets = vec![&t.env, hub_asset(usdc)];
    let _ = t.ctrl_client().get_market_indexes_detailed(&assets);
}

// Fail-closed: the anchor is required. A missing anchor spot (`lastprice`)
// reverts `NoLastPrice` (#210) on the view path; there is no fallback to the
// primary TWAP.
#[test]
#[should_panic(expected = "Error(Contract, #210)")]
fn test_dual_anchor_missing_spot_reverts_on_view() {
    let t = LendingTest::new().dual_source_two_asset();
    let usdc = t.resolve_asset("USDC");

    t.env.as_contract(&t.mock_reflector, || {
        let key = test_harness::mock_reflector::MockKey::Spot(usdc.clone());
        t.env.storage().temporary().remove(&key);
    });

    let assets = vec![&t.env, hub_asset(usdc)];
    let _ = t.ctrl_client().get_market_indexes_detailed(&assets);
}
