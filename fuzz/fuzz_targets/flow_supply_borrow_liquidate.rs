#![no_main]
//! Contract-level libFuzzer target: supply -> borrow -> time/price -> liquidate.
//!
//! Invariants:
//!   - HF > 0 after any successful supply
//!   - `try_liquidate` never panics the host (may return Err for healthy accounts)

use libfuzzer_sys::fuzz_target;
use stellar_fuzz::{build_min_context, ALICE};

fuzz_target!(|data: &[u8]| {
    // Pull parameters from raw bytes so libFuzzer can make progress on tiny
    // seeds rather than rejecting inputs via Arbitrary.
    let b = |i: usize| data.get(i).copied().unwrap_or(0);
    let supply = 1_000.0 + (b(0) as f64) * 500.0;          // 1k..~128k USDC
    let borrow_frac = (b(1) as f64 / 255.0) * 0.9;         // 0..0.9
    let jump = (b(2) as u64) * 3_600;                       // 0..~10.6 days
    let liq_frac = b(3) as f64 / 255.0;                     // 0..1.0

    let mut t = build_min_context();
    t.supply(ALICE, "USDC", supply);
    assert!(t.health_factor(ALICE) > 0.0);

    let max_eth = supply * 0.75 / 2000.0;
    let borrow_amt = max_eth * borrow_frac;
    if borrow_amt > 0.0 {
        let _ = t.try_borrow(ALICE, "ETH", borrow_amt);
    }

    if jump > 0 {
        t.advance_and_sync(jump);
    }

    let debt = t.borrow_balance(ALICE, "ETH");
    if debt > 0.0 && liq_frac > 0.0 {
        let _ = t.try_liquidate("liquidator", ALICE, "ETH", debt * liq_frac);
    }
});
