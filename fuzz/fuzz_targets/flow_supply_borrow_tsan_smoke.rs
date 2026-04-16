#![no_main]
//! Minimal contract-level smoke target used to probe whether libFuzzer links
//! on macOS with `--sanitizer=thread` (see fuzz/README.md).

use libfuzzer_sys::fuzz_target;
use stellar_fuzz::{build_min_context, ALICE};

fuzz_target!(|data: &[u8]| {
    // Derive a tiny supply amount from the input (1..=1000 USDC).
    let amt = 1u64 + (data.first().copied().unwrap_or(0) as u64);

    let mut t = build_min_context();
    t.supply(ALICE, "USDC", amt as f64);

    // Sanity: health factor is reported and positive for a pure supply.
    let hf = t.health_factor(ALICE);
    assert!(hf > 0.0, "health factor must be positive after supply");
});
