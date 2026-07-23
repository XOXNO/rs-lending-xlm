//! Within-tx HF memory attribution: cost is per-call VM frames, not payload bytes.
//! Bulk RedStone + `bulk_get_indexes` cut per-feed slope under one frame.
//! Experiments: total feed slope (withdraw HF), pool-only (`update_indexes`),
//! Reflector Twap(3) vs Twap(12). Mocks undercount absolute wasm size; slopes hold.

extern crate std;

use test_harness::{hub_asset, usdc_preset, LendingTest, ALICE};

fn mem_of<R>(env: &soroban_sdk::Env, f: impl FnOnce() -> R) -> u64 {
    // reset_tracker (NOT reset_default): zero the meters without restoring
    // default limits — restored limits re-enable enforcement and the auth
    // phase panics with Budget,ExceededLimit on multi-feed ops.
    env.cost_estimate().budget().reset_tracker();
    f();
    env.cost_estimate().budget().memory_bytes_cost()
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
