//! Contract-level property test: failed-op atomicity (cache Drop correctness).
//!
//! Invariant: when an op reverts, observable pool state for the calling user
//! must be unchanged. Indexes may advance (global_sync is a separate concern)
//! but reserves and user balances must not drift.

use proptest::prelude::*;
use soroban_sdk::Vec;
use test_harness::{eth_preset, usdc_preset, LendingTest, ALICE};

fn capture_indexes(t: &LendingTest, asset: &str) -> (i128, i128) {
    let asset_addr = t.resolve_asset(asset);
    let mut assets = Vec::new(&t.env);
    assets.push_back(asset_addr);
    let views = t.ctrl_client().get_all_market_indexes_detailed(&assets);
    let v = views.get(0).unwrap();
    (v.supply_index_ray, v.borrow_index_ray)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn prop_failed_borrow_leaves_state_intact(
        supply_usdc in 1_000u64..50_000u64,
        borrow_eth in 1u64..10_000u64,
    ) {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(eth_preset())
            .build();

        t.supply(ALICE, "USDC", supply_usdc as f64);

        let reserves_before = t.pool_reserves("USDC");
        let supply_balance_before = t.supply_balance_raw(ALICE, "USDC");
        let idx_before = capture_indexes(&t, "USDC");

        let borrow_result = t.try_borrow(ALICE, "ETH", borrow_eth as f64);

        if borrow_result.is_err() {
            // Op reverted → USDC-side state must be unchanged
            let reserves_after = t.pool_reserves("USDC");
            let supply_balance_after = t.supply_balance_raw(ALICE, "USDC");
            let idx_after = capture_indexes(&t, "USDC");

            prop_assert!(
                (reserves_before - reserves_after).abs() < 0.0001,
                "USDC reserves drifted on failed borrow: {} -> {}",
                reserves_before, reserves_after
            );
            // On a failed borrow, the user's USDC supply balance must be EXACTLY
            // unchanged (allowing ≤1 raw asset unit for half-up rounding).
            // A looser `>=` check would silently hide a mint bug where supply grows.
            let supply_diff = (supply_balance_after - supply_balance_before).abs();
            prop_assert!(
                supply_diff <= 1,
                "supply drifted on failed borrow: {} -> {} (diff {})",
                supply_balance_before, supply_balance_after, supply_diff
            );
            // Indexes monotonic (may have advanced from global_sync)
            prop_assert!(idx_after.0 >= idx_before.0);
            prop_assert!(idx_after.1 >= idx_before.1);
        }
    }
}
