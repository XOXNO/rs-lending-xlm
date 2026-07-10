use crate::config::config;
use crate::strategy_helpers::{flash_guard_cleared, router_allowance};
use controller::constants::WAD;
use controller::types::PositionMode;
use proptest::prelude::*;
use soroban_sdk::Bytes;
use test_harness::{build_aggregator_swap, usdc_preset, usdt_stable_preset, LendingTest, ALICE};

proptest! {
    #![proptest_config(config(16))]

    #[test]
    fn prop_multiply_succeeds_with_safe_hf_and_clean_router(
        debt_units in 1u32..10u32,
        out_ratio_bps in 15_000u32..50_000u32,
    ) {
        let mut t = LendingTest::new().standard_two_asset().build();
        let eth_amount = debt_units as f64;
        let usdc_out = eth_amount * 2_000.0 * (out_ratio_bps as f64 / 10_000.0);
        t.fund_router("USDC", usdc_out);

        let usdc_decimals = t.resolve_market("USDC").decimals;
        let eth_decimals = t.resolve_market("ETH").decimals;
        let min_out_raw = test_harness::f64_to_i128(usdc_out, usdc_decimals);
        let amount_in_raw = test_harness::f64_to_i128(eth_amount, eth_decimals);
        let steps = build_aggregator_swap(&t, "ETH", "USDC", amount_in_raw, min_out_raw);

        let result = t.try_multiply(
            ALICE,
            "USDC",
            eth_amount,
            "ETH",
            PositionMode::Multiply,
            &steps,
        );
        prop_assert!(result.is_ok(), "valid multiply failed: {:?}", result.err());

        let account_id = t.resolve_account_id(ALICE);
        let hf = t.ctrl_client().get_health_factor(&account_id);
        prop_assert!(hf >= WAD, "HF below 1 after multiply: {}", hf);
        prop_assert_eq!(router_allowance(&t, "ETH"), 0);
        prop_assert!(flash_guard_cleared(&t));
    }

    #[test]
    fn prop_swap_collateral_conserves_position_delta(
        withdraw_units in 100u32..5_000u32,
    ) {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(usdt_stable_preset())
            .build();
        t.supply(ALICE, "USDC", 10_000.0);

        let withdraw_amount = withdraw_units as f64;
        t.fund_router("USDT", withdraw_amount);

        let usdt_decimals = t.resolve_market("USDT").decimals;
        let usdc_decimals = t.resolve_market("USDC").decimals;
        let min_out_raw = test_harness::f64_to_i128(withdraw_amount, usdt_decimals);
        let amount_in_raw = test_harness::f64_to_i128(withdraw_amount, usdc_decimals);
        let steps = build_aggregator_swap(&t, "USDC", "USDT", amount_in_raw, min_out_raw);
        let usdc_before = t.supply_balance_raw(ALICE, "USDC");

        let result = t.try_swap_collateral(ALICE, "USDC", withdraw_amount, "USDT", &steps);
        prop_assert!(result.is_ok(), "valid collateral swap failed: {:?}", result.err());
        prop_assert_eq!(
            t.supply_balance_raw(ALICE, "USDC"),
            usdc_before - amount_in_raw
        );
        prop_assert_eq!(t.supply_balance_raw(ALICE, "USDT"), min_out_raw);
        prop_assert_eq!(router_allowance(&t, "USDC"), 0);
        prop_assert!(flash_guard_cleared(&t));
    }
}

#[test]
fn empty_swap_payload_reverts_without_state_or_guard_leak() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    let usdc_before = t.supply_balance_raw(ALICE, "USDC");

    let result = t.try_swap_collateral(ALICE, "USDC", 1_000.0, "USDT", &Bytes::new(&t.env));

    assert!(result.is_err(), "empty swap payload must be rejected");
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), usdc_before);
    assert_eq!(t.supply_balance_raw(ALICE, "USDT"), 0);
    assert_eq!(router_allowance(&t, "USDC"), 0);
    assert!(flash_guard_cleared(&t));
}
