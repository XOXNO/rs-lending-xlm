use crate::config::config;
use controller::constants::WAD;
use controller::types::PositionMode;
use proptest::prelude::*;
use test_harness::{LendingTest, ALICE};

fn is_budget_panic(msg: &str) -> bool {
    let low = msg.to_lowercase();
    low.contains("budget")
        || low.contains("exceeded")
        || low.contains("limit")
        || low.contains("cpu")
        || low.contains("memory")
}

proptest! {
    #![proptest_config(config(4))]

    #[test]
    fn prop_strategy_under_budget(
        supply_u in 100u32..10_000,
        leverage_bps in 10_000u32..30_000,
    ) {
        let mut t = LendingTest::new().three_asset_usdc_eth_wbtc_with_budget().build();
        t.fund_router("ETH", 1_000_000.0);
        let _ = t.get_or_create_user(ALICE);

        let borrow_eth =
            (supply_u as f64) * (leverage_bps as f64 - 10_000.0) / 10_000.0 / 2_000.0;
        if borrow_eth < 0.0001 {
            return Ok(());
        }

        let steps = t.mock_swap_steps("ETH", "USDC", WAD);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            t.try_multiply(ALICE, "USDC", borrow_eth, "ETH", PositionMode::Normal, &steps)
        }));

        match result {
            Ok(Ok(_)) => {
                prop_assert!(
                    t.health_factor(ALICE) > 0.0,
                    "successful multiply must leave a live position"
                );
            }
            Ok(Err(_)) => {}
            Err(payload) => {
                let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                    (*s).to_string()
                } else if let Some(s) = payload.downcast_ref::<std::string::String>() {
                    s.clone()
                } else {
                    std::string::String::from("<non-string panic payload>")
                };
                prop_assert!(
                    is_budget_panic(&msg),
                    "multiply panicked outside budget category: {}",
                    msg
                );
            }
        }
    }
}
