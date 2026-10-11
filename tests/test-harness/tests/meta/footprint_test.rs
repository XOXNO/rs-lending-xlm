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

// Mainnet per-transaction limits, from soroban-sdk 28's mainnet profile
// (`soroban-sdk-28.0.0/src/testutils/cost_estimate.rs:146-151`, a snapshot of
// the network settings of 2026-07-10). The earlier 50-write and 100-entry
// figures predate SDK 28 and were stale.
const MAINNET_LEDGER_ENTRIES: u32 = 400;
const MAINNET_DISK_READ_ENTRIES: u32 = 200;
const MAINNET_WRITE_ENTRIES: u32 = 200;
const MAINNET_DISK_READ_BYTES: u32 = 200_000;
const MAINNET_WRITE_BYTES: u32 = 132_096;
const MAINNET_EVENT_BYTES: u32 = 16_384;

/// Prints the footprint and asserts that it fits the mainnet per-transaction limits.
fn assert_res(env: &soroban_sdk::Env, label: &str) {
    let r = env.cost_estimate().resources();
    let total = r.disk_read_entries + r.memory_read_entries + r.write_entries;
    std::println!("  {:<45} entries={:>3}/{MAINNET_LEDGER_ENTRIES}  disk_reads={:>3}/{MAINNET_DISK_READ_ENTRIES}  writes={:>2}/{MAINNET_WRITE_ENTRIES}  read_bytes={:>6}/{MAINNET_DISK_READ_BYTES}  write_bytes={:>5}/{MAINNET_WRITE_BYTES}  events={:>5}/{MAINNET_EVENT_BYTES}",
        label, total, r.disk_read_entries, r.write_entries, r.disk_read_bytes, r.write_bytes, r.contract_events_size_bytes);
    assert!(
        total <= MAINNET_LEDGER_ENTRIES,
        "{label}: {total} footprint entries exceed the mainnet {MAINNET_LEDGER_ENTRIES}-entry limit"
    );
    assert!(
        r.disk_read_entries <= MAINNET_DISK_READ_ENTRIES,
        "{label}: {} disk-read entries exceed the mainnet {MAINNET_DISK_READ_ENTRIES}-entry limit",
        r.disk_read_entries
    );
    assert!(
        r.write_entries <= MAINNET_WRITE_ENTRIES,
        "{label}: {} write entries exceed the mainnet {MAINNET_WRITE_ENTRIES}-write limit",
        r.write_entries
    );
    assert!(
        r.disk_read_bytes <= MAINNET_DISK_READ_BYTES,
        "{label}: {} read bytes exceed the mainnet 200KB limit",
        r.disk_read_bytes
    );
    assert!(
        r.write_bytes <= MAINNET_WRITE_BYTES,
        "{label}: {} write bytes exceed the mainnet 132KB limit",
        r.write_bytes
    );
    assert!(
        r.contract_events_size_bytes <= MAINNET_EVENT_BYTES,
        "{label}: {} event bytes exceed the mainnet 16KB limit",
        r.contract_events_size_bytes
    );
}

#[test]
fn footprints_fit_mainnet_limits() {
    std::println!("\n=== FOOTPRINT ANALYSIS (mainnet limits: entries={MAINNET_LEDGER_ENTRIES}, writes={MAINNET_WRITE_ENTRIES}, read=200KB, write=132KB) ===\n");

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
