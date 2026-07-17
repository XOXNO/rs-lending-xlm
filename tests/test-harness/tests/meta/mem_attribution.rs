//! Within-tx HF memory attribution: cost is per-call VM frames, not payload bytes.
//! Bulk RedStone + `bulk_get_indexes` cut per-feed slope under one frame.
//! Experiments: total feed slope (withdraw HF), pool-only (`update_indexes`),
//! Reflector Twap(3) vs Twap(12). Mocks undercount absolute wasm size; slopes hold.

extern crate std;

use controller::types::{ControllerKey, MarketOracleConfig, OracleReadMode, OracleSourceConfig};
use test_harness::{
    eth_preset, hub_asset, usdc_preset, usdt_stable_preset, wbtc_preset, xlm_preset, LendingTest,
    ALICE,
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
        let key = ControllerKey::AssetOracle(asset.clone());
        let mut oracle: MarketOracleConfig = t.env.storage().persistent().get(&key).unwrap();
        if let OracleSourceConfig::Reflector(ref mut src) = oracle.primary {
            src.read_mode = OracleReadMode::Twap(records);
        }
        t.env.storage().persistent().set(&key, &oracle);
    });
}

#[test]
fn mem_attribution_bulk_prefetch_removes_per_feed_pool_frame() {
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
    std::println!(
        "  pool call (fat snapshot, no oracle) ~{pool_per_asset} B/asset  ({:.0}% of total)",
        pool_per_asset as f64 * 100.0 / total_per_feed as f64
    );
    std::println!("  reflector Vec<PriceData> return:    ~{per_record} B/record -> Twap(3) ~{reflector_twap3_per_feed} B/feed  ({:.0}% of total)",
        reflector_twap3_per_feed as f64 * 100.0 / total_per_feed as f64);
    std::println!(
        "  returns accounted together          ~{accounted} B/feed  ({:.0}% of total)",
        accounted as f64 * 100.0 / total_per_feed as f64
    );
    std::println!("  (Twap(12) 5-feed withdraw = {mem_5_feeds_twap12} B)");

    // The claims under test:
    // 1. The un-bulked per-asset pool path still pays a call frame per asset —
    //    the reference slope that makes bulking worthwhile.
    assert!(
        pool_per_asset > 100_000,
        "per-asset update_indexes slope should remain call-frame scale"
    );
    // 2. Bulk index prefetch: HF per-feed slope stays below one pool call frame.
    assert!(
        total_per_feed < pool_per_asset,
        "HF per-feed slope must stay below one pool call frame: \
         total {total_per_feed} vs pool frame {pool_per_asset}"
    );
    // 3. Return payloads still scale with record count.
    assert!(
        mem_5_feeds_twap12 > mem_5_feeds + 5 * 9 * 32,
        "memory must scale with returned record count"
    );
}

/// Direct measurement for "does `Client::new` itself consume memory":
/// constructs the pool client 10,000 times WITHOUT calling anything.
/// The generated `new()` is two handle clones into a 3-word struct
/// (soroban-sdk-macros derive_client.rs) — there is no runtime ABI object;
/// method bodies are static code in the module, loaded once per tx.
#[test]
fn mem_attribution_client_new_is_free() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 1_000.0);
    let pool_addr = t.markets.get("USDC").expect("market").pool.clone();
    let env = t.env.clone();

    let mem_once = mem_of(&env, || {
        std::hint::black_box(pool::LiquidityPoolClient::new(&env, &pool_addr));
    });
    let mem_10k = mem_of(&env, || {
        for _ in 0..10_000 {
            std::hint::black_box(pool::LiquidityPoolClient::new(&env, &pool_addr));
        }
    });
    // One real CALL for contrast (callee wasm instance memory).
    let asset = t.markets.get("USDC").expect("market").asset.clone();
    let key = hub_asset(asset);
    let client = pool::LiquidityPoolClient::new(&env, &pool_addr);
    let mem_call = mem_of(&env, || std::hint::black_box(client.get_reserves(&key)));

    std::println!("\n========== Client::new cost ==========");
    std::println!("  1     x ::new()           mem = {mem_once} B");
    std::println!("  10000 x ::new()           mem = {mem_10k} B");
    std::println!("  1     x actual call       mem = {mem_call} B");

    // 1x and 10,000x constructions meter byte-identically: the marginal
    // memory cost of a Client::new is exactly zero. (The shared baseline is
    // measurement-context overhead; the real call's cost is the callee
    // instance, consistent with the per-call slope measured above.)
    assert_eq!(
        mem_once, mem_10k,
        "9,999 extra constructions must not move the memory meter"
    );
    assert!(mem_call > 100_000, "a real call must dwarf construction");
}
