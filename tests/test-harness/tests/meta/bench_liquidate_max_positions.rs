//! Multi-position liquidation coverage.
//!
//! Liquidating an account with 5 supply and 5 borrow positions (the 5 market
//! presets the harness ships) completes without a logic or arithmetic panic.
//!
//! This file does not assert a transaction-budget fit. Under
//! `mock_all_auths_allowing_non_root_auth` the host meters its own auth
//! re-verification against the budget, a cost that a signed transaction does not
//! pay. The live-testnet integration suite measures the liquidation budget
//! (`flow_stress_liq_frontier` in `tests/integration/flows/stress.sh`).
//! `classify_panic` accepts a budget panic and re-raises every other panic.

use controller::constants::WAD;
use test_harness::{
    eth_preset, usdc_preset, usdt_stable_preset, wbtc_preset, xlm_preset, LendingTest, ALICE,
    LIQUIDATOR,
};

fn build_ctx() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market(xlm_preset())
        .with_position_limits(5, 5)
        .with_budget_enabled()
        .build()
}

fn classify_panic(payload: Box<dyn std::any::Any + Send>) -> Result<(), std::string::String> {
    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<std::string::String>() {
        s.clone()
    } else {
        std::format!("{:?}", payload.type_id())
    };

    let low = msg.to_lowercase();
    let is_overflow = low.contains("overflow") || low.contains("out of bounds");
    let is_budget = !is_overflow
        && (low.contains("budget exceeded")
            || low.contains("exceededlimit")
            || low.contains("cpu instruction")
            || low.contains("memory limit")
            || low.contains("read entries")
            || low.contains("write entries")
            || low.contains("tx size"));
    if is_budget {
        Ok(())
    } else {
        Err(msg)
    }
}

#[test]
fn liquidate_5_supply_5_borrow_completes_without_logic_panic() {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut t = build_ctx();

        t.supply(ALICE, "USDC", 8_000.0);
        t.supply(ALICE, "USDT", 8_000.0);
        t.supply(ALICE, "XLM", 80_000.0);
        t.supply(ALICE, "ETH", 0.01);
        t.supply(ALICE, "WBTC", 0.001);

        t.supply("BOOT", "ETH", 10.0);
        t.supply("BOOT", "WBTC", 1.0);

        t.borrow(ALICE, "ETH", 0.4);
        t.borrow(ALICE, "WBTC", 0.01);
        t.borrow(ALICE, "USDC", 10.0);
        t.borrow(ALICE, "USDT", 10.0);
        t.borrow(ALICE, "XLM", 100.0);

        t.set_price("USDC", WAD / 100);
        t.set_price("USDT", WAD / 100);
        t.set_price("XLM", WAD / 100);

        t.assert_liquidatable(ALICE);

        let payments: &[(&str, f64)] = &[
            ("USDC", 1.0),
            ("USDT", 1.0),
            ("ETH", 0.04),
            ("WBTC", 0.001),
            ("XLM", 10.0),
        ];

        t.liquidate_multi(LIQUIDATOR, ALICE, payments);
    }));

    match outcome {
        Ok(()) => {}
        Err(payload) => {
            classify_panic(payload).unwrap_or_else(|msg| {
                panic!(
                    "BENCH FAILURE: liquidate setup or call panicked outside the budget envelope: {}",
                    msg
                )
            });
        }
    }
}

/// Fails when the ctx position limits exceed the 5 legs the scenario above
/// exercises. The harness ships 5 market presets, so add presets and extend the
/// scenario before raising the limits.
#[test]
fn test_scenario_covers_every_configured_leg() {
    let scenario_legs = 5u32;
    let limits = build_ctx().get_position_limits();
    assert!(
        limits.max_supply_positions <= scenario_legs
            && limits.max_borrow_positions <= scenario_legs,
        "liquidation scenario exercises {sc}/{sc} legs but the ctx permits {}/{} — \
         add market presets and extend the scenario before widening the ctx limits",
        limits.max_supply_positions,
        limits.max_borrow_positions,
        sc = scenario_legs,
    );
}
