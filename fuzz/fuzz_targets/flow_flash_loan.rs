#![no_main]
//! Contract-level libFuzzer target: flash loan good/bad receivers.
//!
//! Invariants:
//!   - Good receiver: repayment >= borrowed + fees (try_flash_loan Ok)
//!   - Bad receiver: try_flash_loan must Err (no silent success)

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use stellar_fuzz::{arb_amount, assert_global_invariants, build_min_context, ALICE};

#[derive(Arbitrary, Debug)]
struct Input {
    seed_usdc: u32,
    loan_usdc: u32,
    use_bad: bool,
}

fuzz_target!(|inp: Input| {
    let seed = arb_amount(inp.seed_usdc, 10_000.0, 110_000.0);
    let loan = arb_amount(inp.loan_usdc, 1.0, 50_001.0);

    let mut t = build_min_context();
    t.supply(ALICE, "USDC", seed);
    assert_global_invariants(&t, ALICE, &["USDC", "ETH"], 0.0);

    if inp.use_bad {
        let bad = t.deploy_bad_flash_loan_receiver();
        let res = t.try_flash_loan(ALICE, "USDC", loan, &bad);
        assert!(res.is_err(), "bad receiver flash loan must fail");
    } else {
        let good = t.deploy_flash_loan_receiver();
        let _ = t.try_flash_loan(ALICE, "USDC", loan, &good);
    }
});
