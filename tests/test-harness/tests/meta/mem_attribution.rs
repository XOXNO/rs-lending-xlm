extern crate std;

use test_harness::{hub_asset, usdc_preset, LendingTest, ALICE};

fn mem_of<R>(env: &soroban_sdk::Env, f: impl FnOnce() -> R) -> u64 {
    env.cost_estimate().budget().reset_tracker();
    f();
    env.cost_estimate().budget().memory_bytes_cost()
}

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

    let asset = t.markets.get("USDC").expect("market").asset.clone();
    let key = hub_asset(asset);
    let client = pool::LiquidityPoolClient::new(&env, &pool_addr);
    let mem_call = mem_of(&env, || std::hint::black_box(client.get_reserves(&key)));

    std::println!("\n========== Client::new cost ==========");
    std::println!("  1     x ::new()           mem = {mem_once} B");
    std::println!("  10000 x ::new()           mem = {mem_10k} B");
    std::println!("  1     x actual call       mem = {mem_call} B");

    assert_eq!(
        mem_once, mem_10k,
        "9,999 extra constructions must not move the memory meter"
    );
    assert!(mem_call > 100_000, "a real call must dwarf construction");
}
