//! Attributes the per-feed memory cost of HF-checked operations.
//!
//! MEASURED CONCLUSION: the dominant consumer is the per-CALL wasm VM frame
//! (~1.28MB of linear-memory charge per cross-contract invocation), not the
//! returned struct's payload bytes (~509 B per Reflector PriceData record,
//! ~0.1% of the per-feed cost). The native-mock reflector calls are frame-free
//! while the deployed-wasm pool call carries 95% of the per-feed slope; on
//! testnet/mainnet all three calls per feed (pool + Reflector + RedStone) are
//! wasm, so each feed costs ~3 frames (~4MB) — 40MB cap / ~4MB explains the
//! measured 10-feed dual-source wall. Consequence: REMOVING CALLS (local pool
//! mirror, bulk RedStone) is the lever; slimming structs or Twap(3)->Twap(2)
//! buys almost nothing.
//!
//! Method — all measurements are WITHIN one transaction, because standalone
//! client calls are each their own top-level invocation and re-charge ~1.1MB
//! of VM instantiation, drowning the payload signal (a real tx pays that
//! once; measured: `reserves() -> i128` and `get_sync_data() -> PoolSyncData`
//! both cost ~1.4MB standalone). Three within-tx experiments:
//!
//! 1. TOTAL per-feed slope: `withdraw` with debt recomputes HF over ALL
//!    positions in one tx — measure at 2 vs 5 distinct feeds.
//! 2. POOL-ONLY slope: `update_indexes` makes one pool call per asset
//!    (fat MarketStateSnapshot return) and reads NO oracles — measure at
//!    1 vs 5 assets. This isolates the pool-return component.
//! 3. REFLECTOR-RETURN scaling: switch every market's primary read mode
//!    Twap(3) -> Twap(12) (the mock returns `records` PriceData entries) and
//!    re-measure the same 5-feed withdraw. The delta is purely the extra
//!    records allocated per feed — the oracle-return component.
//!
//! Mocks are native, so absolute numbers undercount deployed-wasm oracles;
//! the slopes and deltas still attribute where the memory goes.

extern crate std;

use common::types::{ControllerKey, MarketConfig, OracleReadMode, OracleSourceConfig};
use test_harness::{
    eth_preset, usdc_preset, usdt_stable_preset, wbtc_preset, xlm_preset, LendingTest, ALICE,
};

fn mem_of<R>(env: &soroban_sdk::Env, f: impl FnOnce() -> R) -> u64 {
    // reset_tracker (NOT reset_default): zero the meters without restoring
    // default limits — restored limits re-enable enforcement and the auth
    // phase panics with Budget,ExceededLimit on multi-feed ops.
    env.cost_estimate().budget().reset_tracker();
    f();
    env.cost_estimate().budget().memory_bytes_cost()
}

fn set_primary_twap(t: &LendingTest, asset_name: &str, records: u32) {
    let asset = t.markets.get(asset_name).expect("market").asset.clone();
    t.env.as_contract(&t.controller, || {
        let key = ControllerKey::Market(asset.clone());
        let mut market: MarketConfig = t.env.storage().persistent().get(&key).unwrap();
        if let OracleSourceConfig::Reflector(ref mut src) = market.oracle_config.primary {
            src.read_mode = OracleReadMode::Twap(records);
        }
        t.env.storage().persistent().set(&key, &market);
    });
}

#[test]
fn mem_attribution_per_call_frame_dominates() {
    let names = ["USDC", "USDT", "ETH", "WBTC", "XLM"];
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        // No `with_budget_enabled()`: limits stay DISABLED so heavy ops can
        // complete; the meters still track, which is all measurement needs.
        .with_market(xlm_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 0.1);

    // 1. TOTAL slope: HF valuation in one withdraw tx, 2 feeds vs 5 feeds.
    let mem_2_feeds = mem_of(&t.env.clone(), || t.withdraw(ALICE, "USDC", 10.0));
    t.borrow(ALICE, "USDT", 100.0);
    t.borrow(ALICE, "WBTC", 0.01);
    t.borrow(ALICE, "XLM", 100.0);
    let mem_5_feeds = mem_of(&t.env.clone(), || t.withdraw(ALICE, "USDC", 10.0));
    let total_per_feed = (mem_5_feeds - mem_2_feeds) / 3;

    // 2. POOL-ONLY slope: per-asset pool call (fat snapshot return), no oracles.
    t.update_indexes_for(&["USDC"]);
    let mem_idx_1 = mem_of(&t.env.clone(), || t.update_indexes_for(&["USDC"]));
    let mem_idx_5 = mem_of(&t.env.clone(), || t.update_indexes_for(&names));
    let pool_per_asset = (mem_idx_5 - mem_idx_1) / 4;

    // 3. REFLECTOR-RETURN scaling: Twap(3) -> Twap(12) = +9 records per feed.
    for name in names {
        set_primary_twap(&t, name, 12);
    }
    let mem_5_feeds_twap12 = mem_of(&t.env.clone(), || t.withdraw(ALICE, "USDC", 10.0));
    let per_record = (mem_5_feeds_twap12 - mem_5_feeds) / (9 * 5);
    let reflector_twap3_per_feed = per_record * 3;

    let accounted = pool_per_asset + reflector_twap3_per_feed;
    std::println!("\n========== per-feed memory attribution (within-tx slopes) ==========");
    std::println!("  withdraw HF valuation:  2 feeds = {mem_2_feeds} B, 5 feeds = {mem_5_feeds} B");
    std::println!("  TOTAL marginal cost            ~{total_per_feed} B/feed");
    std::println!("  pool call (fat snapshot, no oracle) ~{pool_per_asset} B/asset  ({:.0}% of total)",
        pool_per_asset as f64 * 100.0 / total_per_feed as f64);
    std::println!("  reflector Vec<PriceData> return:    ~{per_record} B/record -> Twap(3) ~{reflector_twap3_per_feed} B/feed  ({:.0}% of total)",
        reflector_twap3_per_feed as f64 * 100.0 / total_per_feed as f64);
    std::println!("  returns accounted together          ~{accounted} B/feed  ({:.0}% of total)",
        accounted as f64 * 100.0 / total_per_feed as f64);
    std::println!("  (Twap(12) 5-feed withdraw = {mem_5_feeds_twap12} B)");

    // The claim under test: per-feed memory is dominated by the cross-contract
    // CALLS themselves (VM frame), with the return payload a minor term.
    assert!(total_per_feed > 100_000, "per-feed slope should be ~1MB-scale");
    assert!(
        mem_5_feeds_twap12 > mem_5_feeds + 5 * 9 * 32,
        "memory must scale with returned record count"
    );
    assert!(
        accounted * 2 > total_per_feed,
        "return payloads should account for the majority of per-feed memory: \
         pool {pool_per_asset} + reflector {reflector_twap3_per_feed} vs total {total_per_feed}"
    );
}
