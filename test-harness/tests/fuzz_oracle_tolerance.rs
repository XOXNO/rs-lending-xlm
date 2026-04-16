//! Contract-level property test: oracle tolerance tier enforcement.
//!
//! Invariants:
//!   - Supply (risk-decreasing) always succeeds, regardless of safe-vs-spot
//!     deviation.
//!   - Borrow (risk-increasing) either succeeds with safe price, uses avg,
//!     or reverts — never panics unexpectedly.

use proptest::prelude::*;
use test_harness::{eth_preset, helpers::usd, usdc_preset, LendingTest, ALICE};

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn prop_supply_always_works_under_deviation(
        supply_amt in 1_000u64..100_000u64,
        deviation_bps in 0u16..5_000u16,
        direction_up in any::<bool>(),
    ) {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(eth_preset())
            .build();

        // First supply at matching prices to seed the account.
        t.supply(ALICE, "USDC", supply_amt as f64);

        // Desynchronize spot vs TWAP on ETH.
        let eth_spot = usd(2000);
        let mult = if direction_up {
            10_000 + deviation_bps as i128
        } else {
            (10_000 - deviation_bps as i128).max(1)
        };
        let eth_twap = eth_spot * mult / 10_000;

        let reflector = t.mock_reflector_client();
        let eth_addr = t.resolve_asset("ETH");
        reflector.set_price(&eth_addr, &eth_spot);
        reflector.set_twap_price(&eth_addr, &eth_twap);

        // Supply (risk-decreasing) must succeed regardless of deviation.
        let res = t.try_supply(ALICE, "USDC", 1.0);
        prop_assert!(
            res.is_ok(),
            "supply rejected under deviation {} bps dir_up={}",
            deviation_bps, direction_up
        );
    }

    /// Stale oracle prices must cause supply to revert.
    ///
    /// Contract behavior (controller/src/oracle/mod.rs):
    ///   `check_staleness` panics with `OracleError::PriceFeedStale` when
    ///   `(now - feed_ts) > max_stale` (default 900 s).
    #[test]
    fn prop_supply_rejects_stale_price(
        supply_amt in 1_000u64..100_000u64,
        stale_seconds in 1_000_000u64..5_000_000u64,
    ) {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(eth_preset())
            .build();

        // Seed USDC supply while prices are fresh so the account exists.
        t.supply(ALICE, "USDC", supply_amt as f64);

        // Advance the ledger past max_price_stale_seconds without refreshing
        // oracle prices: every price now carries a stale timestamp.
        t.advance_time_no_refresh(stale_seconds);

        let res = t.try_supply(ALICE, "USDC", 1.0);
        prop_assert!(
            res.is_err(),
            "supply accepted despite stale price (advanced {} s)",
            stale_seconds
        );
    }

    /// A zero-valued oracle price must cause supply to revert.
    ///
    /// Contract behavior (controller/src/oracle/mod.rs line 42-44):
    ///   `if price <= 0 { panic OracleError::InvalidPrice }`.
    #[test]
    fn prop_supply_rejects_zero_price(
        supply_amt in 1_000u64..100_000u64,
    ) {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(eth_preset())
            .build();

        // Seed USDC supply at normal prices first.
        t.supply(ALICE, "USDC", supply_amt as f64);

        // Flip USDC spot + TWAP to zero — any subsequent oracle read must
        // panic with `InvalidPrice` and propagate as a try_* Err.
        let reflector = t.mock_reflector_client();
        let usdc_addr = t.resolve_asset("USDC");
        reflector.set_price(&usdc_addr, &0);
        reflector.set_twap_price(&usdc_addr, &0);

        let res = t.try_supply(ALICE, "USDC", 1.0);
        prop_assert!(
            res.is_err(),
            "supply accepted despite zero oracle price"
        );
    }
}
