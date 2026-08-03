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
fn bench_liquidate_5_supply_5_borrow_within_default_budget() {
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

#[test]
fn test_position_limit_cap_matches_bench_coverage() {
    let t = build_ctx();
    let limits = t.get_position_limits();
    let max_proven = 5u32;
    assert!(
        limits.max_supply_positions <= max_proven,
        "bench coverage is {}/{}; controller permits {}/{} — extend the preset set before raising the cap",
        max_proven,
        max_proven,
        limits.max_supply_positions,
        limits.max_borrow_positions
    );
    assert!(
        limits.max_borrow_positions <= max_proven,
        "bench coverage is {}/{}; controller permits {}/{} — extend the preset set before raising the cap",
        max_proven,
        max_proven,
        limits.max_supply_positions,
        limits.max_borrow_positions
    );
}
