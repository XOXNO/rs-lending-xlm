use common::types::{ControllerKey, MarketConfig, OracleReadMode, OracleSourceConfig};
use soroban_sdk::testutils::Events as _;
use soroban_sdk::vec;
use test_harness::{assert_contract_error, errors, usd_cents, LendingTest, ALICE};

fn setup() -> LendingTest {
    LendingTest::new().dual_source_two_asset()
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

// Permissive policy (supply / repay) falls through `twap_fallback_or_panic`
// → `read_spot_from_env`. Drives the permissive arm of the helper.
#[test]
fn test_empty_twap_history_falls_back_to_spot_on_supply() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &2);

    // Supply is risk-decreasing → `twap_fallback_or_panic` resolves to the
    // spot path rather than reverting.
    t.supply(ALICE, "USDC", 10_000.0);
    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
}

#[test]
fn test_insufficient_twap_history_falls_back_to_spot_on_supply() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &3);

    t.supply(ALICE, "USDC", 10_000.0);
    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
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
    // The reader emits `OracleError::InvalidPrice` via the
    // `has_invalid_price` branch.
    assert!(
        result.is_err(),
        "borrow should fail when a TWAP entry has non-positive price"
    );
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
    assert!(
        result.is_err(),
        "borrow should fail when TWAP window contains a stale timestamp"
    );
}

// Mode 4 under permissive (supply) policy: `newest_valid` is `Some` (one
// of the entries was valid), so `twap_fallback_or_panic` returns the
// newest valid observation — exercises the `Some(_)` arm of the
// `fallback.or_else(...)` chain in `twap_fallback_or_panic`.
#[test]
fn test_twap_invalid_price_falls_back_to_newest_valid_on_supply() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &4);

    // Risk-decreasing path → permissive fallback should return the
    // newest valid sample rather than reverting.
    t.supply(ALICE, "USDC", 10_000.0);
    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
}

#[test]
fn test_twap_stale_history_falls_back_on_supply() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &5);

    t.supply(ALICE, "USDC", 10_000.0);
    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
}

// Permissive policy + degraded TWAP must emit `oracle/twap_degraded` so
// operators can detect silent fallback to spot. The `View` policy allows
// every loosening; reading market indexes after marking USDC's TWAP as
// empty drives `twap_fallback_or_panic` through its fallback arm and the
// event must surface.
#[test]
fn test_twap_degradation_emits_oracle_event_on_view() {
    let t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &2);

    let assets = soroban_sdk::Vec::from_array(&t.env, [usdc_asset]);
    let _ = t
        .ctrl_client()
        .get_all_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();

    let dump = format!("{:#?}", t.env.events().all());
    assert!(
        dump.contains("twap_degraded"),
        "permissive view on stale TWAP must emit `twap_degraded`; events were:\n{}",
        dump
    );
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
        test_harness::DEFAULT_TOLERANCE.first_upper_bps,
        test_harness::DEFAULT_TOLERANCE.last_upper_bps,
    );
    t.ctrl_client()
        .configure_market_oracle(&t.admin(), &eth_asset, &spot_cfg);

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

// PR-5: `read_twap` with `records == 0` hits the early fallback arm (twap.rs:29-36).
#[test]
fn test_twap_zero_records_falls_back_on_permissive_view() {
    let t = LendingTest::new().dual_source_two_asset();
    let usdc = t.resolve_asset("USDC");
    t.env.as_contract(&t.controller, || {
        let key = ControllerKey::Market(usdc.clone());
        let mut market: MarketConfig = t.env.storage().persistent().get(&key).unwrap();
        if let OracleSourceConfig::Reflector(ref mut source) = market.oracle_config.primary {
            source.read_mode = OracleReadMode::Twap(0);
        }
        t.env.storage().persistent().set(&key, &market);
    });

    let assets = vec![&t.env, usdc.clone()];
    let view = t
        .ctrl_client()
        .get_all_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    assert!(
        view.price_wad > 0,
        "Twap(0) must fall back to anchor spot on view"
    );
}

// PR-5: `records > MAX_TWAP_RECORDS` hits the guard in `read_twap` (twap.rs:38-41).
#[test]
#[should_panic(expected = "Error(Contract, #204)")]
fn test_twap_records_above_max_rejects_on_view() {
    let t = LendingTest::new().dual_source_two_asset();
    let usdc = t.resolve_asset("USDC");
    t.env.as_contract(&t.controller, || {
        let key = ControllerKey::Market(usdc.clone());
        let mut market: MarketConfig = t.env.storage().persistent().get(&key).unwrap();
        if let OracleSourceConfig::Reflector(ref mut source) = market.oracle_config.primary {
            source.read_mode = OracleReadMode::Twap(13);
        }
        t.env.storage().persistent().set(&key, &market);
    });

    let assets = vec![&t.env, usdc];
    let _ = t.ctrl_client().get_all_market_indexes_detailed(&assets);
}

// PR-10: anchor spot read with `required=false` returns None when lastprice is missing.
#[test]
fn test_dual_anchor_missing_spot_falls_back_to_primary_on_view() {
    let t = LendingTest::new().dual_source_two_asset();
    let usdc = t.resolve_asset("USDC");

    t.env.as_contract(&t.mock_reflector, || {
        let key = test_harness::mock_reflector::MockKey::Spot(usdc.clone());
        t.env.storage().temporary().remove(&key);
    });

    let assets = vec![&t.env, usdc.clone()];
    let view = t
        .ctrl_client()
        .get_all_market_indexes_detailed(&assets)
        .get(0)
        .unwrap();
    assert!(
        view.price_wad > 0,
        "view must fall back to primary TWAP, not revert"
    );
}
