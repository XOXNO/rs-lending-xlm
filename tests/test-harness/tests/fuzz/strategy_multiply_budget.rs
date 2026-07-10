use crate::config::config;
use crate::strategy_helpers::{flash_guard_cleared, router_allowance};
use controller::types::PositionMode;
use proptest::prelude::*;
use test_harness::{build_aggregator_swap, LendingTest, ALICE};

proptest! {
    #![proptest_config(config(4))]

    #[test]
    fn prop_valid_multiply_fits_default_budget(
        debt_tenths in 1u32..50,
        collateral_ratio_bps in 15_000u32..20_000,
    ) {
        let mut t = LendingTest::new()
            .standard_two_asset()
            .with_budget_enabled()
            .build();

        let debt_eth = debt_tenths as f64 / 10.0;
        let collateral_usdc = debt_eth * 2_000.0 * collateral_ratio_bps as f64 / 10_000.0;
        t.fund_router("USDC", collateral_usdc);

        let eth_decimals = t.resolve_market("ETH").decimals;
        let usdc_decimals = t.resolve_market("USDC").decimals;
        let amount_in_raw = test_harness::f64_to_i128(debt_eth, eth_decimals);
        let min_out_raw = test_harness::f64_to_i128(collateral_usdc, usdc_decimals);
        let steps = build_aggregator_swap(&t, "ETH", "USDC", amount_in_raw, min_out_raw);

        // Setup calls are not part of the transaction-equivalent operation.
        t.env.cost_estimate().budget().reset_default();
        let result = t.try_multiply(
            ALICE,
            "USDC",
            debt_eth,
            "ETH",
            PositionMode::Multiply,
            &steps,
        );
        prop_assert!(
            result.is_ok(),
            "valid multiply exceeded the default budget or failed: {:?}",
            result.err()
        );
        prop_assert!(t.health_factor_raw(ALICE) >= controller::constants::WAD);
        prop_assert_eq!(router_allowance(&t, "ETH"), 0);
        prop_assert!(flash_guard_cleared(&t));
    }
}
