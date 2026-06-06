use crate::config::config;
use crate::strategy_helpers::{flash_guard_cleared, router_allowance};
use common::constants::WAD;
use common::types::PositionMode;
use proptest::prelude::*;
use soroban_sdk::Bytes;
use test_harness::{
    build_aggregator_swap, usdc_preset, usdt_stable_preset, LendingTest, ALICE,
};

proptest! {
    #![proptest_config(config(16))]

    #[test]
    fn prop_multiply_leverage_hf_safe(
        debt_units in 1u32..10u32,
        out_ratio_bps in 15_000u32..50_000u32,
    ) {
        let mut t = LendingTest::new().standard_two_asset().build();
        let eth_amount = debt_units as f64;
        let usdc_out = eth_amount * 2_000.0 * (out_ratio_bps as f64 / 10_000.0);
        t.fund_router("USDC", usdc_out);

        let usdc_decimals = t.resolve_market("USDC").decimals;
        let eth_decimals = t.resolve_market("ETH").decimals;
        let min_out_raw = (usdc_out as i128) * 10i128.pow(usdc_decimals);
        let amount_in_raw = (eth_amount as i128) * 10i128.pow(eth_decimals);
        let steps = build_aggregator_swap(&t, "ETH", "USDC", amount_in_raw, min_out_raw);

        let result = t.try_multiply(ALICE, "USDC", eth_amount, "ETH", PositionMode::Multiply, &steps);

        prop_assert_eq!(router_allowance(&t, "ETH"), 0);
        prop_assert!(flash_guard_cleared(&t));

        match result {
            Ok(account_id) => {
                let hf = t.ctrl_client().health_factor(&account_id);
                prop_assert!(hf >= WAD, "HF below 1 after multiply: {}", hf);
            }
            Err(_) => {
                let active = t.get_active_accounts(ALICE);
                prop_assert_eq!(active.len(), 0, "failed multiply leaked an account");
            }
        }
    }

    #[test]
    fn prop_strategy_swap_collateral_balance_delta(
        withdraw_frac_bps in 100u32..5_000u32,
        payload_valid in any::<bool>(),
    ) {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(usdt_stable_preset())
            .build();
        t.supply(ALICE, "USDC", 10_000.0);

        let withdraw_amount = 10_000.0 * (withdraw_frac_bps as f64) / 10_000.0;
        t.fund_router("USDT", withdraw_amount * 2.0);

        let usdt_decimals = t.resolve_market("USDT").decimals;
        let usdc_decimals = t.resolve_market("USDC").decimals;
        let min_out_raw = if payload_valid {
            (withdraw_amount as i128) * 10i128.pow(usdt_decimals)
        } else {
            0
        };
        let amount_in_raw = (withdraw_amount as i128) * 10i128.pow(usdc_decimals);
        let steps = if payload_valid {
            build_aggregator_swap(&t, "USDC", "USDT", amount_in_raw, min_out_raw)
        } else {
            Bytes::new(&t.env)
        };

        let result = t.try_swap_collateral(ALICE, "USDC", withdraw_amount, "USDT", &steps);

        if !payload_valid {
            prop_assert!(result.is_err(), "empty swap payload must be rejected");
        } else if result.is_ok() {
            prop_assert!(t.supply_balance(ALICE, "USDT") > 0.0);
            prop_assert_eq!(router_allowance(&t, "USDC"), 0);
        }
        prop_assert!(flash_guard_cleared(&t));
    }
}