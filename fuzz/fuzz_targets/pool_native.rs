//! Native-Rust fuzz target for `pool::LiquidityPool`.
//!
//! Registers the pool as a **native** Soroban contract (via `env.register`)
//! rather than loading its WASM bytecode. This is the ONLY way to get native
//! coverage instrumentation (`-Cinstrument-coverage`) to see the pool code —
//! `flow_e2e` / `flow_strategy` load compiled WASM and execute it in
//! Soroban's VM, which bypasses native profile counters entirely.
//!
//! Scope: functions reachable *without* token transfers. `supply` / `borrow`
//! / `withdraw` / `repay` all invoke `token::Client::transfer` against
//! `params.asset_id`; wiring a Stellar Asset Contract here would add a lot of
//! setup code for little marginal coverage value. That layer is already
//! exercised via the WASM path in `flow_e2e`. Instead, we focus on:
//!
//!   - `__constructor` (initial state)
//!   - `update_indexes(price_wad)` (interest accrual pipeline —
//!     exercises `pool/src/interest.rs`, `pool/src/cache.rs`)
//!   - `add_rewards(price_wad, amount)` (rewards accrual path)
//!   - All view functions: `capital_utilisation`, `reserves`,
//!     `deposit_rate`, `borrow_rate`, `protocol_revenue`,
//!     `supplied_amount`, `borrowed_amount`, `delta_time`, `get_sync_data`
//!   - `keepalive` (TTL extension path)
//!
//! Invariants asserted each iteration:
//!   - `supplied_amount >= borrowed_amount` (supply floor)
//!   - `reserves >= 0`
//!   - `borrow_index` monotonically non-decreasing across `update_indexes`
//!   - `supply_index` monotonically non-decreasing across `update_indexes`
//!   - `deposit_rate <= borrow_rate` (supplier APY ≤ borrower APY)
#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, RAY, WAD};
use common::types::MarketParams;
use libfuzzer_sys::fuzz_target;
use pool::{LiquidityPool, LiquidityPoolClient};
use soroban_sdk::{testutils::Address as _, testutils::Ledger as _, Address, Env};

#[derive(Debug, Arbitrary)]
struct In {
    // Interest-curve geometry (same clamping as rates_and_index).
    base_pct: u8,
    s1_pct: u8,
    s2_pct: u8,
    s3_pct: u16,
    mid_pct: u8,
    opt_pct: u8,
    max_pct: u16,
    reserve_pct: u8,
    // Sequence of (price_wad, time_advance_ms, op_kind) ops.
    // op_kind dispatches: update_indexes / add_rewards / keepalive / view-only.
    ops: [(u32, u32, u8); 8],
}

fn make_params(_env: &Env, asset: &Address, i: &In) -> MarketParams {
    let mid_pct = (i.mid_pct % 98 + 1) as i128;
    let opt_pct = (i.opt_pct as i128 % (99 - mid_pct)) + mid_pct + 1;

    MarketParams {
        base_borrow_rate_ray: RAY * (i.base_pct as i128 % 51) / 100,
        slope1_ray: RAY * (i.s1_pct as i128 % 51) / 100,
        slope2_ray: RAY * (i.s2_pct as i128 % 101) / 100,
        slope3_ray: RAY * (i.s3_pct as i128 % 501) / 100,
        mid_utilization_ray: RAY * mid_pct / 100,
        optimal_utilization_ray: RAY * opt_pct / 100,
        max_borrow_rate_ray: (RAY * (i.max_pct.max(1) as i128 % 1001) / 100).max(1),
        reserve_factor_bps: ((i.reserve_pct as i128 % 51) * 100).clamp(0, BPS - 1),
        asset_id: asset.clone(),
        asset_decimals: 7,
    }
}

fuzz_target!(|i: In| {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);

    // Register a real Stellar Asset Contract so view functions like
    // `reserves()` (which calls `asset_token.balance(pool)`) succeed
    // instead of panicking with `Storage, MissingValue`. No tokens are
    // actually minted — the SAC returns balance 0 for the pool, which is
    // fine for the pre-activity invariants we assert.
    let asset = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address()
        .clone();

    let params = make_params(&env, &asset, &i);

    // Register the pool natively at a fresh address. Running
    // `__constructor` via register's second argument keeps the contract's
    // instance storage populated.
    let pool_addr = env.register(LiquidityPool, (admin, params.clone()));
    let pool = LiquidityPoolClient::new(&env, &pool_addr);

    // Baseline: supplied/borrowed are both zero, indices at RAY.
    assert_eq!(pool.supplied_amount(), 0);
    assert_eq!(pool.borrowed_amount(), 0);
    assert_eq!(pool.reserves(), 0);

    let mut prev_borrow_index: i128 = RAY;
    let mut prev_supply_index: i128 = RAY;
    // Track ledger time in seconds — Soroban's TestLedger timestamp is seconds.
    let mut cur_ts_s: u64 = env.ledger().timestamp();

    for (price_raw, dt_raw, op_kind) in i.ops.iter() {
        // Price: clamped to a realistic wad range [0.001, 1000].
        let price_wad: i128 = ((*price_raw as i128 % 1_000_000) + 1).saturating_mul(WAD / 1_000);

        // Time advance: up to 100 days per step (scaled from u32).
        let dt_s: u64 = (*dt_raw as u64) % (100 * 86_400);
        cur_ts_s = cur_ts_s.saturating_add(dt_s);
        env.ledger().set_timestamp(cur_ts_s);

        match op_kind % 4 {
            0 => {
                // update_indexes — interest accrual path. Use try_* so
                // rejected calls (e.g. math overflow on extreme inputs)
                // don't crash the harness.
                if let Ok(Ok(idx)) = pool.try_update_indexes(&price_wad) {
                    assert!(
                        idx.borrow_index_ray >= prev_borrow_index,
                        "borrow index regressed: prev={} new={}",
                        prev_borrow_index,
                        idx.borrow_index_ray
                    );
                    assert!(
                        idx.supply_index_ray >= prev_supply_index,
                        "supply index regressed: prev={} new={}",
                        prev_supply_index,
                        idx.supply_index_ray
                    );
                    prev_borrow_index = idx.borrow_index_ray;
                    prev_supply_index = idx.supply_index_ray;
                }
            }
            1 => {
                // add_rewards — fails with NoSuppliersToReward (#37) when
                // supplied == 0. Expected; swallow via try_*.
                let amount = ((*price_raw as i128) % 10_000_000) + 1;
                let _ = pool.try_add_rewards(&price_wad, &amount);
            }
            2 => {
                // keepalive — TTL extension path.
                let _ = pool.try_keepalive();
            }
            _ => {
                // Pure-view sweep — read-only functions shouldn't fail
                // under fresh-pool state; assert cross-function invariants.
                let util = pool.capital_utilisation();
                let reserves = pool.reserves();
                let deposit = pool.deposit_rate();
                let borrow = pool.borrow_rate();
                let rev = pool.protocol_revenue();
                let supplied = pool.supplied_amount();
                let borrowed = pool.borrowed_amount();
                let _dt = pool.delta_time();
                let _sync = pool.get_sync_data();

                assert!(
                    supplied >= borrowed,
                    "supplied ({}) < borrowed ({})",
                    supplied,
                    borrowed
                );
                assert!(reserves >= 0, "negative reserves: {}", reserves);
                assert!(rev >= 0, "negative protocol revenue: {}", rev);
                assert_eq!(util, 0, "non-zero utilisation with no activity: {}", util);
                assert!(
                    deposit <= borrow + 1,
                    "deposit rate > borrow rate: dep={} bor={}",
                    deposit,
                    borrow
                );
            }
        }
    }
});
