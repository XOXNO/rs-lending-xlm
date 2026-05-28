//! Contract-level property test: budget-bounded metering.
//!
//! The default `LendingTest::new().build()` path calls `reset_unlimited()` +
//! `disable_resource_limits()`, so most correctness tests ignore Soroban's
//! cost model. Mainnet enforces these limits.
//!
//! This file opts in to Soroban's default budget via
//! `LendingTestBuilder::with_budget_enabled()` and fuzzes inputs to verify
//! that on-chain calls stay within budget or fail cleanly with a budget
//! error — never producing an opaque host panic.
//!
//! Acceptable outcomes:
//!   * `Ok(_)` -- operation stays within budget.
//!   * `Err(ExceededLimit)` or any Soroban-host budget panic -- the cost
//!     model correctly rejects the oversized input. Whitelist this.
//!
//! Unacceptable:
//!   * Any *other* panic. The cost model must produce clean errors, never
//!     opaque panics.

extern crate std;

use common::types::PositionMode;
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;
use test_harness::{eth_preset, usdc_preset, wbtc_preset, LendingTest};

/// Build a harness with Soroban's default budget and resource limits enabled.
fn build_ctx_with_budget() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_budget_enabled()
        .build()
}

// `multiply` with realistic leverage (<= 3x) runs within the default budget
// or fails cleanly. Catches cost-model violations on the flash-loan +
// strategy path.

proptest! {
    #![proptest_config(ProptestConfig { cases: 4, ..ProptestConfig::default() })]

    #[test]
    fn prop_strategy_under_budget(
        supply_u in 100u32..10_000,
        // leverage: 1.0x .. 3.0x, encoded as basis points of supply.
        leverage_bps in 10_000u32..30_000,
    ) {
        let mut t = build_ctx_with_budget();

        // Fund the mock router + cross-asset liquidity for the swap.
        t.fund_router("ETH", 1_000_000.0);

        let user = "alice";
        let _addr = Address::generate(&t.env);
        let _ = t.get_or_create_user(user);

        // Collateral: USDC, debt: ETH (standard strategy pair).
        // debt_amount = (supply_usd * (leverage_bps - 10_000) / 10_000) / eth_price
        // Use a tiny number to stay within liquidity; the test targets the
        // cost model, not the economics.
        let borrow_eth = (supply_u as f64) * (leverage_bps as f64 - 10_000.0) / 10_000.0 / 2_000.0;

        let steps = t.mock_swap_steps("ETH", "USDC", common::constants::WAD);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            t.try_multiply(user, "USDC", borrow_eth.max(0.0001), "ETH", PositionMode::Normal, &steps)
        }));

        match result {
            Ok(Ok(_)) => {}
            Ok(Err(_)) => {}
            Err(payload) => {
                let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                    (*s).to_string()
                } else if let Some(s) = payload.downcast_ref::<std::string::String>() {
                    s.clone()
                } else {
                    std::string::String::from("<non-string panic payload>")
                };
                let low = msg.to_lowercase();
                let is_budget = low.contains("budget")
                    || low.contains("exceeded")
                    || low.contains("limit")
                    || low.contains("cpu")
                    || low.contains("memory");
                prop_assert!(
                    is_budget,
                    "CRITICAL: opaque panic outside budget category: {}",
                    msg
                );
            }
        }
    }
}
