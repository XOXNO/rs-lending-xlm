#![no_main]
//! Contract-level libFuzzer target: stateful multi-op sequence.
//!
//! Replaces the retired `flow_cache_atomicity` / `flow_isolation_emode_xor` /
//! `flow_supply_borrow_tsan_smoke` targets. libFuzzer mutates an `Op`
//! sequence; after every step we enforce two classes of invariants:
//!
//!   1. On success — global invariants (HF floor + reserves ≥ 0). The HF
//!      floor is `1.0` after risk-increasing ops (borrow/withdraw) and
//!      `0.0` after risk-decreasing ops (supply/repay). This matches the
//!      proptest harness's post-op gate.
//!
//!   2. On failure — reserves + user raw balances must be unchanged (the
//!      cache-Drop atomicity property the retired `flow_cache_atomicity`
//!      target used to assert). Silent state drift on a reverted `try_*`
//!      call is a bug.
//!
//! Ops intentionally exercise the under-fuzzed `withdraw` / `repay` paths
//! that the liquidation flow only hits transitively.

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use stellar_fuzz::{
    arb_amount, assert_global_invariants, assert_state_preserved_on_failure, build_min_context,
    snapshot, HF_WAD_FLOOR, ALICE,
};

const ASSETS: [&str; 2] = ["USDC", "ETH"];

#[derive(Arbitrary, Debug)]
enum Op {
    Supply { asset: u8, amount: u32 },
    Borrow { asset: u8, amount: u32 },
    Withdraw { asset: u8, amount: u32 },
    Repay { asset: u8, amount: u32 },
    Advance { hours: u16 },
}

#[derive(Arbitrary, Debug)]
struct Input {
    ops: Vec<Op>,
}

fn pick(asset: u8) -> &'static str {
    ASSETS[(asset as usize) % ASSETS.len()]
}

fuzz_target!(|inp: Input| {
    let ops: Vec<_> = inp.ops.into_iter().take(16).collect();
    if ops.is_empty() {
        return;
    }

    let mut t = build_min_context();
    t.supply(ALICE, "USDC", 10_000.0);
    assert_global_invariants(&t, ALICE, &ASSETS, 0.0);

    for op in ops {
        // Snapshot BEFORE every fallible op so we can detect failure-path
        // state drift (replaces flow_cache_atomicity's assertion).
        let before = snapshot(&t, ALICE, &ASSETS);

        let (ok, min_hf_on_success) = match op {
            Op::Supply { asset, amount } => {
                let a = pick(asset);
                let amt = arb_amount(amount, 1.0, 50_000.0);
                (t.try_supply(ALICE, a, amt).is_ok(), 0.0)
            }
            Op::Borrow { asset, amount } => {
                let a = pick(asset);
                let amt = arb_amount(amount, 1.0, 10_000.0);
                (t.try_borrow(ALICE, a, amt).is_ok(), HF_WAD_FLOOR)
            }
            Op::Withdraw { asset, amount } => {
                let a = pick(asset);
                let amt = arb_amount(amount, 1.0, 50_000.0);
                (t.try_withdraw(ALICE, a, amt).is_ok(), HF_WAD_FLOOR)
            }
            Op::Repay { asset, amount } => {
                let a = pick(asset);
                let amt = arb_amount(amount, 1.0, 10_000.0);
                (t.try_repay(ALICE, a, amt).is_ok(), 0.0)
            }
            Op::Advance { hours } => {
                let secs = (hours as u64) * 3_600;
                if secs > 0 {
                    t.advance_and_sync(secs);
                }
                // Time advance is infallible and risk-neutral for invariants.
                (true, 0.0)
            }
        };

        if ok {
            assert_global_invariants(&t, ALICE, &ASSETS, min_hf_on_success);
        } else {
            // Failed op must not have mutated pool/user state.
            let after = snapshot(&t, ALICE, &ASSETS);
            assert_state_preserved_on_failure(&before, &after);
        }
    }
});
