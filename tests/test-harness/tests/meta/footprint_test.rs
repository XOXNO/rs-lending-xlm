use test_harness::presets::{
    MarketPreset, ALICE, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, LIQUIDATOR,
};
use test_harness::{helpers::usd, LendingTest};

fn mk(name: &'static str, dec: u32, price: i128, liq: f64) -> MarketPreset {
    MarketPreset {
        name,
        decimals: dec,
        price_wad: price,
        initial_liquidity: liq,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// Prints the footprint AND enforces the mainnet limits it is measured
/// against — a scenario crossing a limit is the regression this file exists
/// to catch, and used to report it in green.
fn assert_res(env: &soroban_sdk::Env, label: &str) {
    let r = env.cost_estimate().resources();
    let total = r.disk_read_entries + r.memory_read_entries + r.write_entries;
    std::println!("  {:<45} entries={:>3}/100  writes={:>2}/50  read_bytes={:>6}/200000  write_bytes={:>5}/132096  events={:>5}/16384",
        label, total, r.write_entries, r.disk_read_bytes, r.write_bytes, r.contract_events_size_bytes);
    assert!(
        total <= 100,
        "{label}: {total} footprint entries exceed the mainnet 100-entry limit"
    );
    assert!(
        r.write_entries <= 50,
        "{label}: {} write entries exceed the mainnet 50-write limit",
        r.write_entries
    );
    assert!(
        r.disk_read_bytes <= 200_000,
        "{label}: {} read bytes exceed the mainnet 200KB limit",
        r.disk_read_bytes
    );
    assert!(
        r.write_bytes <= 132_096,
        "{label}: {} write bytes exceed the mainnet 132KB limit",
        r.write_bytes
    );
    assert!(
        r.contract_events_size_bytes <= 16_384,
        "{label}: {} event bytes exceed the mainnet 16KB limit",
        r.contract_events_size_bytes
    );
}

#[test]
fn footprints_fit_mainnet_limits() {
    std::println!("\n=== FOOTPRINT ANALYSIS (mainnet limits: entries=100, writes=50, read=200KB, write=132KB) ===\n");

    {
        let mut t = LendingTest::new()
            .with_market(mk("USDC", 6, usd(1), 1_000_000.0))
            .build();
        t.supply(ALICE, "USDC", 10_000.0);
        assert_res(&t.env, "Supply (1 market)");
    }

    {
        let mut t = LendingTest::new()
            .with_market(mk("USDC", 6, usd(1), 1_000_000.0))
            .with_market(mk("DAI", 18, usd(1), 1_000_000.0))
            .build();
        t.supply(ALICE, "USDC", 10_000.0);
        t.borrow(ALICE, "DAI", 5_000.0);
        assert_res(&t.env, "Borrow + HF check (2 markets)");
    }

    {
        let mut t = LendingTest::new()
            .with_market(mk("USDC", 6, usd(1), 1_000_000.0))
            .with_market(mk("DAI", 18, usd(1), 1_000_000.0))
            .build();
        t.supply(ALICE, "USDC", 10_000.0);
        t.borrow(ALICE, "DAI", 7_500.0);
        t.set_price("USDC", usd(1) * 85 / 100);
        t.advance_and_sync(1000);
        t.liquidate(LIQUIDATOR, ALICE, "DAI", 2_000.0);
        assert_res(&t.env, "Liquidation 1C+1D (2 markets)");
    }

    {
        let mut t = LendingTest::new()
            .with_market(mk("USDC", 6, usd(1), 1_000_000.0))
            .with_market(mk("DAI", 18, usd(1), 1_000_000.0))
            .with_market(mk("WBTC", 8, usd(60_000), 100_000.0))
            .build();
        t.supply(ALICE, "USDC", 5_000.0);
        let a = t.resolve_account_id(ALICE);
        t.supply_to(ALICE, a, "DAI", 5_000.0);
        t.borrow(ALICE, "WBTC", 0.125);
        t.set_price("USDC", usd(1) * 85 / 100);
        t.set_price("DAI", usd(1) * 85 / 100);
        t.advance_and_sync(1000);
        t.liquidate(LIQUIDATOR, ALICE, "WBTC", 0.03);
        assert_res(&t.env, "Liquidation 2C+1D (3 markets)");
    }

    {
        let mut t = LendingTest::new()
            .with_market(mk("A", 6, usd(1), 1_000_000.0))
            .with_market(mk("B", 18, usd(1), 1_000_000.0))
            .with_market(mk("C", 8, usd(60_000), 100_000.0))
            .with_market(mk("D", 9, usd(150), 100_000.0))
            .build();
        t.supply(ALICE, "A", 5_000.0);
        let a = t.resolve_account_id(ALICE);
        t.supply_to(ALICE, a, "B", 5_000.0);
        t.borrow(ALICE, "C", 0.058);
        t.borrow(ALICE, "D", 23.0);
        t.set_price("A", usd(1) * 85 / 100);
        t.set_price("B", usd(1) * 85 / 100);
        t.advance_and_sync(1000);
        t.liquidate_multi(LIQUIDATOR, ALICE, &[("C", 0.01), ("D", 5.0)]);
        assert_res(&t.env, "Liquidation 2C+2D (4 markets)");
    }

    std::println!();
}
