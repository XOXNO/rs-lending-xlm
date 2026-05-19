//! Coverage for the TWAP-edge branches in
//! `controller/src/oracle/providers/reflector.rs`. Each test exercises one
//! of the mock reflector's history modes (`set_twap_history_mode`) under a
//! permissive policy (supply) so the `twap_fallback_or_panic` path resolves
//! to the spot-fallback rather than reverting.
//!
//! Modes (see `verification/test-harness/src/mock_reflector.rs`):
//!   0 = normal
//!   1 = None (history call returns None)
//!   2 = empty (history call returns an empty Vec)
//!   3 = insufficient (history call returns fewer records than requested)

extern crate std;

use test_harness::{
    assert_contract_error, errors, eth_preset, usd_cents, usdc_preset, LendingTest, ALICE,
};

fn setup() -> LendingTest {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();
    t.set_oracle_primary_anchor("USDC");
    t.set_oracle_primary_anchor("ETH");
    // Seed history-derived prices so the TWAP path has something to consume.
    t.set_safe_price("USDC", common::constants::WAD, true, true);
    t.set_safe_price("ETH", common::constants::WAD * 2_000, true, true);
    t
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
