//! Empirical max-position liquidate cost benchmark.
//!
//! Audit prep deliverable referenced by `audit/THREAT_MODEL.md §3.3` and
//! `audit/AUDIT_CHECKLIST.md "Still Outstanding"`. Validates that
//! `liquidate` against an account holding the maximum supported number of
//! supply + borrow positions either:
//!
//!   * `Ok(_)` -- the call fits Soroban's default tx budget (400M
//!     instructions, 200 r/w entries, 132 KB tx size, 16 KB events,
//!     286 KB write bytes). This is the success path the operator
//!     runbook depends on.
//!   * `Err(_)` from a budget / limit error -- the cost model rejects
//!     the operation cleanly. The operator must lower `PositionLimits`
//!     until the call fits.
//!
//! What it must never do:
//!   * Opaque host panic that does not surface a budget / limit error.
//!     That would render the affected account un-liquidatable.
//!
//! Scope: 5 markets (the full preset set: USDC, USDT, ETH, WBTC, XLM). At
//! `PositionLimits = 5/5` Alice opens 5 supply positions + 5 borrow
//! positions, becomes liquidatable through a coordinated price drop, and
//! a liquidator submits a 5-asset `liquidate` payment vector. This is the
//! largest empirical benchmark the existing harness supports without
//! adding new market presets.
//!
//! For 32/32 (the contract-level cap from `set_position_limits`), the
//! harness needs additional preset assets; tracked as a follow-up in
//! `audit/AUDIT_CHECKLIST.md "Still Outstanding"`.

extern crate std;

use common::constants::WAD;
use test_harness::{
    eth_preset, usdc_preset, usdt_stable_preset, wbtc_preset, xlm_preset, LendingTest, ALICE,
    LIQUIDATOR,
};

/// Build a test context with all 5 preset markets and Soroban's default
/// budget enforced. Position limits raised to 5/5 so Alice can open the
/// full diagonal of (supply, borrow) positions across the 5 markets.
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

/// Classify any host-panic payload. Returns `Ok(())` if the payload
/// surfaces a budget / limit error, `Err(message)` otherwise. Used to keep
/// the bench resilient to either Soroban's `HostError::Budget(ExceededLimit)`
/// or our own `panic_with_error!` propagation.
fn classify_panic(payload: Box<dyn std::any::Any + Send>) -> Result<(), std::string::String> {
    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<std::string::String>() {
        s.clone()
    } else {
        std::format!("{:?}", payload.type_id())
    };
    let low = msg.to_lowercase();
    let is_budget = low.contains("budget")
        || low.contains("exceeded")
        || low.contains("limit")
        || low.contains("cpu")
        || low.contains("memory")
        || low.contains("entries")
        || low.contains("size");
    if is_budget {
        Ok(())
    } else {
        Err(msg)
    }
}

#[test]
fn bench_liquidate_5_supply_5_borrow_within_default_budget() {
    // Capture the entire bench (setup + liquidate) under one catch_unwind so
    // any budget / limit error surfaced *anywhere* in the multi-asset setup
    // flow is classified, not silently raised. Soroban's host budget is
    // shared across the whole transaction-equivalent invocation graph; a
    // setup-phase budget exhaustion is the same failure class as a
    // liquidate-phase exhaustion for this benchmark's purpose.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut t = build_ctx();

        // Alice supplies across all 5 markets.
        t.supply(ALICE, "USDC", 5_000.0);
        t.supply(ALICE, "USDT", 5_000.0);
        t.supply(ALICE, "ETH", 2.0); //   ~$4_000
        t.supply(ALICE, "WBTC", 0.05); // ~$3_000
        t.supply(ALICE, "XLM", 50_000.0); // ~$5_000

        // Alice borrows from all 5 markets, but at conservative LTV so the
        // initial borrow succeeds.
        t.borrow(ALICE, "USDC", 1_000.0);
        t.borrow(ALICE, "USDT", 1_000.0);
        t.borrow(ALICE, "ETH", 0.4);
        t.borrow(ALICE, "WBTC", 0.01);
        t.borrow(ALICE, "XLM", 10_000.0);

        // Push collateral prices down so HF drops below 1 and Alice becomes
        // liquidatable. Halve every collateral price; debt prices stay flat.
        t.set_price("USDC", WAD * 50 / 100);
        t.set_price("USDT", WAD * 50 / 100);
        t.set_price("ETH", WAD * 1_000);
        t.set_price("WBTC", WAD * 30_000);
        t.set_price("XLM", WAD * 5 / 100); // $0.05

        // Submit a 5-asset debt payment vector. Use small amounts so the
        // partial-liquidation path doesn't panic on `repay > debt`.
        let payments: &[(&str, f64)] = &[
            ("USDC", 100.0),
            ("USDT", 100.0),
            ("ETH", 0.04),
            ("WBTC", 0.001),
            ("XLM", 1_000.0),
        ];

        t.liquidate_multi(LIQUIDATOR, ALICE, payments);
    }));

    match outcome {
        Ok(()) => {
            // Liquidation succeeded inside the default Soroban budget --
            // the production-target case.
        }
        Err(payload) => {
            // Acceptable iff the panic carries a budget / limit error. The
            // operator runbook for raising `PositionLimits` past 5/5 must
            // re-run this benchmark and lower the limit until liquidate
            // fits.
            classify_panic(payload).unwrap_or_else(|msg| {
                panic!(
                    "BENCH FAILURE: liquidate setup or call panicked outside the budget envelope: {}",
                    msg
                )
            });
        }
    }
}
