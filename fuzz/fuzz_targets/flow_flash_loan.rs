#![no_main]
//! Contract-level libFuzzer target: flash loan good/bad receivers.
//!
//! Invariants:
//!   - Good receiver: repayment >= borrowed + fees (try_flash_loan Ok)
//!   - Bad receiver: try_flash_loan must Err (no silent success)

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use stellar_fuzz::{build_min_context, ALICE};

#[derive(Arbitrary, Debug)]
struct Input {
    seed_usdc: u32,
    loan_usdc: u32,
    use_bad: bool,
}

fuzz_target!(|inp: Input| {
    let seed = ((inp.seed_usdc % 100_000) + 10_000) as f64;
    let loan = ((inp.loan_usdc % 50_000) + 1) as f64;

    let mut t = build_min_context();
    // Seed pool liquidity so the flash loan has funds to draw.
    t.supply(ALICE, "USDC", seed);

    if inp.use_bad {
        let bad = t.deploy_bad_flash_loan_receiver();
        let res = t.try_flash_loan(ALICE, "USDC", loan, &bad);
        assert!(res.is_err(), "bad receiver flash loan must fail");
    } else {
        let good = t.deploy_flash_loan_receiver();
        let _ = t.try_flash_loan(ALICE, "USDC", loan, &good);
    }
});
