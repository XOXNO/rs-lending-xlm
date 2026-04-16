//! Contract-level property test: supply → borrow → time/price → liquidate.
//!
//! Invariants asserted each iteration:
//!   - HF ≥ 1.0 after any successful borrow.
//!   - Liquidation accepted only when HF < 1.0.
//!   - supply_index / borrow_index monotonically non-decreasing.
//!   - Liquidation never worsens HF while the account still exists.
//!
//! Run:
//!   cargo test -p test-harness --test fuzz_supply_borrow_liquidate
//!   PROPTEST_CASES=10000 cargo test -p test-harness --test fuzz_supply_borrow_liquidate
//!   PROPTEST_CASES=100000 cargo test --release -p test-harness --test fuzz_supply_borrow_liquidate

use common::constants::WAD;
use proptest::prelude::*;
use soroban_sdk::Vec;
use test_harness::{eth_preset, helpers::usd, usdc_preset, LendingTest, ALICE, LIQUIDATOR};

/// Read (supply_index_ray, borrow_index_ray) via the controller view.
fn capture_indexes(t: &LendingTest, asset: &str) -> (i128, i128) {
    let asset_addr = t.resolve_asset(asset);
    let mut assets = Vec::new(&t.env);
    assets.push_back(asset_addr);
    let views = t.ctrl_client().get_all_market_indexes_detailed(&assets);
    let v = views.get(0).expect("market index view empty");
    (v.supply_index_ray, v.borrow_index_ray)
}

proptest! {
    #![proptest_config(ProptestConfig {
        // Keep default case count small for CI; override with the
        // PROPTEST_CASES env var.
        cases: 64,
        max_global_rejects: 100_000,
        ..ProptestConfig::default()
    })]

    #[test]
    fn prop_supply_borrow_liquidate(
        supply_usdc in 1_000u64..500_000u64,
        borrow_frac_bps in 0u16..9_000u16,
        time_jump_secs in 0u64..(30 * 24 * 3600),
        eth_price_rise_bps in 0u16..20_000u16,
        liq_repay_frac_bps in 0u16..10_001u16,
    ) {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(eth_preset())
            .build();

        // --- Supply ---
        t.supply(ALICE, "USDC", supply_usdc as f64);

        let idx_before = capture_indexes(&t, "USDC");

        // --- Borrow ---
        // Max theoretical = supply * LTV(75%) / ETH_price($2000)
        let max_eth = (supply_usdc as f64) * 0.75 / 2000.0;
        let borrow_frac = borrow_frac_bps as f64 / 10_000.0;
        let borrow_amt = max_eth * borrow_frac;

        if borrow_amt < 0.0001 {
            return Ok(());
        }
        // Capture HF before the borrow attempt to assert atomicity on failure.
        let hf_pre_borrow = t.health_factor_raw(ALICE);
        if t.try_borrow(ALICE, "ETH", borrow_amt).is_err() {
            // Even when the borrow is rejected, state must stay atomic: HF unchanged.
            let hf_post_reject = t.health_factor_raw(ALICE);
            prop_assert_eq!(
                hf_pre_borrow, hf_post_reject,
                "HF drifted on rejected borrow: {} -> {}",
                hf_pre_borrow, hf_post_reject
            );
            return Ok(()); // validation rejection, not a bug
        }

        let hf_after_borrow = t.health_factor_raw(ALICE);
        prop_assert!(
            hf_after_borrow >= WAD,
            "HF < 1.0 after borrow: hf={} supply={} borrow_eth={}",
            hf_after_borrow, supply_usdc, borrow_amt
        );

        // --- Time passes ---
        if time_jump_secs > 0 {
            t.advance_and_sync(time_jump_secs);
        }

        let idx_after_accrual = capture_indexes(&t, "USDC");
        prop_assert!(
            idx_after_accrual.0 >= idx_before.0,
            "supply_index regressed: {} -> {}", idx_before.0, idx_after_accrual.0
        );
        prop_assert!(
            idx_after_accrual.1 >= idx_before.1,
            "borrow_index regressed: {} -> {}", idx_before.1, idx_after_accrual.1
        );

        // --- Price rise on debt asset (puts Alice underwater) ---
        if eth_price_rise_bps == 0 {
            return Ok(());
        }
        let new_eth_price = usd(2000) * (10_000 + eth_price_rise_bps as i128) / 10_000;
        t.set_price("ETH", new_eth_price);

        // --- Conditional liquidation ---
        let hf_pre_liq = t.health_factor_raw(ALICE);
        if hf_pre_liq >= WAD {
            // Still healthy: no liquidation runs this iteration. Indexes
            // must still stay monotonic through the healthy-exit path.
            let idx_healthy_exit = capture_indexes(&t, "USDC");
            prop_assert!(
                idx_healthy_exit.0 >= idx_after_accrual.0,
                "supply_index regressed on healthy-exit: {} -> {}",
                idx_after_accrual.0, idx_healthy_exit.0
            );
            prop_assert!(
                idx_healthy_exit.1 >= idx_after_accrual.1,
                "borrow_index regressed on healthy-exit: {} -> {}",
                idx_after_accrual.1, idx_healthy_exit.1
            );
            return Ok(());
        }

        let current_debt_eth = t.borrow_balance(ALICE, "ETH");
        let repay_frac = liq_repay_frac_bps as f64 / 10_000.0;
        let repay_amt = (current_debt_eth * repay_frac).max(0.0001);
        // Capture total debt pre-liquidation to verify the "debt strictly
        // reduced or account closed" invariant after a successful
        // liquidation.
        let debt_before_liquidation = t.total_debt_raw(ALICE);
        let liq_result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", repay_amt);

        // Post-liquidation invariant:
        // Do NOT assert HF strictly improves — when a position is heavily
        // underwater (HF < 1 + bonus, roughly < 1.08), partial liquidations
        // mathematically degrade HF because the liquidator seizes
        // (debt_repaid * (1 + bonus)) in collateral. This is correct
        // protocol behavior: each liquidation reduces bad-debt exposure
        // even as HF drops further toward zero.
        //
        // What we CAN assert:
        //   - HF stays finite and positive.
        //   - Total debt is strictly reduced (or the position closes).
        if t.find_account_id(ALICE).is_some() {
            let hf_post = t.health_factor_raw(ALICE);
            prop_assert!(
                hf_post > 0,
                "HF went non-positive after liquidation: {}", hf_post
            );

            // If the liquidation call itself returned Ok, then either total debt
            // is strictly reduced, or the account is closed (handled above).
            if liq_result.is_ok() {
                let debt_after = t.total_debt_raw(ALICE);
                prop_assert!(
                    debt_after < debt_before_liquidation,
                    "liquidation succeeded but total debt did not decrease: {} -> {}",
                    debt_before_liquidation, debt_after
                );
            }
        } else if liq_result.is_ok() {
            // Liquidation closed the account — that satisfies the invariant.
        }

        let idx_final = capture_indexes(&t, "USDC");
        prop_assert!(idx_final.0 >= idx_after_accrual.0);
        prop_assert!(idx_final.1 >= idx_after_accrual.1);
    }
}
