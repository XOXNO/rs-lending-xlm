#![no_main]
//! Contract-level libFuzzer target: cache Drop correctness on failed ops.
//!
//! Invariant: when `try_borrow` reverts, the USDC-side pool state and the
//! user's supply balance must be (approximately) unchanged.

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use stellar_fuzz::{build_min_context, ALICE};

#[derive(Arbitrary, Debug)]
struct Input {
    supply_usdc: u32,
    borrow_eth: u32,
}

fuzz_target!(|inp: Input| {
    let supply = ((inp.supply_usdc % 50_000) + 1_000) as f64;
    let borrow = ((inp.borrow_eth % 10_000) + 1) as f64;

    let mut t = build_min_context();
    t.supply(ALICE, "USDC", supply);

    let reserves_before = t.pool_reserves("USDC");
    let supply_before = t.supply_balance_raw(ALICE, "USDC");

    let res = t.try_borrow(ALICE, "ETH", borrow);
    if res.is_err() {
        let reserves_after = t.pool_reserves("USDC");
        let supply_after = t.supply_balance_raw(ALICE, "USDC");
        assert!(
            (reserves_before - reserves_after).abs() < 0.0001,
            "USDC reserves drifted on failed borrow"
        );
        assert!(
            (supply_after - supply_before).abs() <= 1,
            "supply drifted on failed borrow"
        );
    }
});
