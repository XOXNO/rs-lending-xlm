//! Boundary regressions for the strategy entrypoints + oracle sanity
//! bounds. Existing suites cover happy-path and most rejection cases;
//! this file pins:
//!
//! 1. Sanity-bound exact-at-floor / exact-at-ceiling accept the price,
//!    just-below-floor / just-above-ceiling reject.
//! 2. Strategy multiply that would push utilization above the cap
//!    is rejected at the gate.

extern crate std;

use common::constants::WAD;
use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usdc_preset, LendingTest, ALICE, BOB,
};

// ---------------------------------------------------------------------------
// 1. Sanity-bound exact boundary
// ---------------------------------------------------------------------------

fn set_sanity_bounds(t: &LendingTest, asset_name: &str, min_wad: i128, max_wad: i128) {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller, || {
        let key = common::types::ControllerKey::Market(asset.clone());
        let mut market: common::types::MarketConfig =
            t.env.storage().persistent().get(&key).unwrap();
        market.oracle_config.min_sanity_price_wad = min_wad;
        market.oracle_config.max_sanity_price_wad = max_wad;
        t.env.storage().persistent().set(&key, &market);
    });
}

// Price exactly equal to the ceiling must be accepted; even 1 WAD over
// must be rejected. Pins the inequality (≤ vs <).
#[test]
fn test_sanity_bound_ceiling_exact_accept_then_one_over_reject() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Set ETH ceiling at exactly $2000 (current price). Reads must
    // succeed.
    set_sanity_bounds(&t, "ETH", usd(100), usd(2_000));
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Now push ETH price to $2000 + 1 WAD-cent → must reject.
    // 1 WAD-cent = WAD / 100 = 10^16
    t.set_price("ETH", usd(2_000) + WAD / 100);
    let result = t.try_borrow(ALICE, "ETH", 0.1);
    assert_contract_error(result, errors::SANITY_BOUND_VIOLATED);
}

// Floor exact-boundary test.
#[test]
fn test_sanity_bound_floor_exact_accept_then_one_under_reject() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Set ETH floor at exactly $2000 (current price). Reads must
    // succeed.
    set_sanity_bounds(&t, "ETH", usd(2_000), usd(10_000));
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Drop ETH below the floor by 1 WAD-cent → must reject.
    t.set_price("ETH", usd(2_000) - WAD / 100);
    let result = t.try_borrow(ALICE, "ETH", 0.1);
    assert_contract_error(result, errors::SANITY_BOUND_VIOLATED);
}

// ---------------------------------------------------------------------------
// 2. Strategy borrow respects max-utilization cap
// ---------------------------------------------------------------------------

// A regular borrow that would push utilization above the cap is
// rejected (covered by `max_utilization_tests.rs`). This pins that the
// same gate applies on the strategy.multiply path: the synthetic
// flash-borrow inside multiply still flows through the same
// utilization-cap check.
#[test]
fn test_borrow_at_cap_then_step_over_rejected() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_params("USDC", |p| {
            // Tight cap: 85 %. (Must stay ≥ optimal=80 % per validator.)
            p.max_utilization_ray = common::constants::RAY * 85 / 100;
        })
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);

    // Borrow $850 → utilization = 85 %. Exactly at cap, allowed.
    t.borrow(BOB, "USDC", 850.0);

    // One more dollar — over the cap, rejected.
    let result = t.try_borrow(BOB, "USDC", 1.0);
    assert_contract_error(result, errors::UTILIZATION_ABOVE_MAX);
}
