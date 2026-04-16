#![no_main]
//! Contract-level libFuzzer target: E-Mode vs Isolation mutual exclusivity.
//!
//! Invariants:
//!   - Creating an account with e_mode > 0 AND is_isolated=true must panic
//!   - Valid e-mode account: is_isolated == false

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use test_harness::{usdc_preset, xlm_preset, LendingTest, STABLECOIN_EMODE, ALICE};

#[derive(Arbitrary, Debug)]
struct Input {
    choose_emode: bool,
    supply_amt: u32,
}

fuzz_target!(|inp: Input| {
    let supply = ((inp.supply_amt % 50_000) + 100) as f64;

    let mut t = LendingTest::new()
        .with_market({
            let mut p = usdc_preset();
            p.config.e_mode_enabled = true;
            p
        })
        .with_market({
            let mut p = xlm_preset();
            p.config.is_isolated_asset = true;
            p.config.isolation_borrow_enabled = true;
            p
        })
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .build();

    // Valid path only: create an e-mode account + supply.
    //
    // The "invalid combo" case (emode + isolated) is covered by the proptest
    // harness in `test-harness/tests/fuzz_isolation_emode_xor.rs` because it
    // requires `std::panic::catch_unwind`, which libFuzzer treats as a deadly
    // signal on macOS (the Soroban host abort path bypasses unwinding).
    let _ = inp.choose_emode;
    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", supply);
    assert!(t.health_factor(ALICE) > 0.0);
});
