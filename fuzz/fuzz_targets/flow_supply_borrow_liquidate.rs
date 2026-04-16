#![no_main]
//! Contract-level libFuzzer target: supply -> borrow -> time/price -> liquidate.
//!
//! Invariants:
//!   - Post-supply: HF > 0, reserves ≥ 0
//!   - Post-borrow (success): HF ≥ 1.0 (proptest-aligned floor)
//!   - Post-accrual: reserves still ≥ 0 (HF may drop below 1 under accrual)
//!   - Post-liquidation: reserves ≥ 0 and HF > 0; `try_liquidate` never panics

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use stellar_fuzz::{
    arb_amount, assert_global_invariants, build_min_context, HF_WAD_FLOOR, ALICE,
};

#[derive(Arbitrary, Debug)]
struct Input {
    supply_raw: u32,
    borrow_frac_raw: u8, // 0..=255 → 0..0.9 of max safe borrow
    jump_hours: u16,     // 0..=65535 hours
    liq_frac_raw: u8,    // 0..=255 → 0..1.0 of outstanding debt
}

const ASSETS: [&str; 2] = ["USDC", "ETH"];

fuzz_target!(|inp: Input| {
    let supply = arb_amount(inp.supply_raw, 1_000.0, 128_000.0);
    let borrow_frac = (inp.borrow_frac_raw as f64 / 255.0) * 0.9;
    let jump = inp.jump_hours as u64 * 3_600;
    let liq_frac = inp.liq_frac_raw as f64 / 255.0;

    let mut t = build_min_context();
    t.supply(ALICE, "USDC", supply);
    assert_global_invariants(&t, ALICE, &ASSETS, 0.0);

    let max_eth = supply * 0.75 / 2000.0;
    let borrow_amt = max_eth * borrow_frac;
    if borrow_amt > 0.0 {
        if t.try_borrow(ALICE, "ETH", borrow_amt).is_ok() {
            assert_global_invariants(&t, ALICE, &ASSETS, HF_WAD_FLOOR);
        }
    }

    if jump > 0 {
        t.advance_and_sync(jump);
        // After accrual, HF may drop below 1 (that's how positions become
        // liquidatable). Only the non-HF invariants must hold.
        assert_global_invariants(&t, ALICE, &ASSETS, 0.0);
    }

    let debt = t.borrow_balance(ALICE, "ETH");
    if debt > 0.0 && liq_frac > 0.0 {
        let _ = t.try_liquidate("liquidator", ALICE, "ETH", debt * liq_frac);
        // Whether or not liquidation succeeded, reserves must stay ≥ 0 and
        // the account must still have a positive HF (post-liquidation HF
        // can be below 1 for heavily underwater positions — see README
        // "What we do NOT assert" for why we don't assert HF ≥ 1 here).
        assert_global_invariants(&t, ALICE, &ASSETS, 0.0);
    }
});
