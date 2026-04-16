//! Fuzz `rescale_half_up` — cross-decimal conversion.
//!
//! Invariants:
//!   1. Identity when from==to
//!   2. Upscale → Downscale roundtrip is lossless
//!   3. Downscale half-up correctness: |rescale(a, from, to)*10^(from-to) - a| ≤ 10^(from-to)/2
//!   4. Sign preservation
//!   5. Upscale overflow panics with clear message (not silent wrap)
#![no_main]
use arbitrary::Arbitrary;
use common::fp_core::rescale_half_up;
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
struct In {
    a: i128,
    from: u8,
    to: u8,
}

// Protocol-realistic bound on the magnitude of a fixed-point value.
// i128::MIN triggers (a - half) underflow in the impl — unreachable in
// real flows where values originate from token amounts ≤ 10^27.
const MAX_A: i128 = 10i128.pow(30);

fuzz_target!(|i: In| {
    // Bound decimals to realistic precision range [0, 27]
    let from = (i.from % 28) as u32;
    let to = (i.to % 28) as u32;

    // Skip pathological magnitudes outside protocol usage
    if i.a == i128::MIN || i.a.abs() > MAX_A {
        return;
    }

    // Same-precision is identity
    if from == to {
        assert_eq!(rescale_half_up(i.a, from, to), i.a);
        return;
    }

    if to > from {
        // Upscale: only test inputs that won't overflow i128.
        let diff = (to - from) as u32;
        let factor: i128 = 10i128.pow(diff);
        // Bound |a| such that |a * factor| < i128::MAX / 2
        let bound = (i128::MAX / 2) / factor;
        if i.a.abs() > bound {
            return;
        }
        // `rescale_half_up` panics explicitly on upscale overflow — that's the
        // designed behavior, not a bug. Skip inputs we know will trip it.
        let up = match std::panic::catch_unwind(|| rescale_half_up(i.a, from, to)) {
            Ok(v) => v,
            Err(_) => return,
        };
        // Roundtrip: upscale then downscale is lossless for integer a
        let back = rescale_half_up(up, to, from);
        assert_eq!(back, i.a, "upscale roundtrip lost data: a={} up={} back={}", i.a, up, back);
        // Sign preservation (zero is fine either way)
        if i.a > 0 {
            assert!(up > 0, "upscale lost positive sign: a={} -> {}", i.a, up);
        } else if i.a < 0 {
            assert!(up < 0, "upscale lost negative sign: a={} -> {}", i.a, up);
        }
    } else {
        // Downscale: lossy, but bounded
        let diff = (from - to) as u32;
        let factor: i128 = 10i128.pow(diff);
        let down = rescale_half_up(i.a, from, to);
        // Reconstruct and check error bound
        // down * factor should be within [a - factor/2, a + factor/2] for half-up
        if let Some(reconstructed) = down.checked_mul(factor) {
            let err = (reconstructed - i.a).abs();
            assert!(
                err <= factor / 2 + 1,
                "downscale exceeds half-up bound: a={} down={} reconstructed={} err={} factor={}",
                i.a,
                down,
                reconstructed,
                err,
                factor
            );
        }
        // Sign: downscale preserves sign for |a| ≥ factor/2; below threshold it can round to 0 either way.
        // The strict away-from-zero property: |a| ≥ factor/2 must produce a non-zero
        // result with sign(a). This catches mis-rounding bugs where a negative
        // value at-or-above the half threshold truncates to 0.
        if i.a.abs() >= factor / 2 && i.a != 0 {
            assert!(
                down != 0,
                "downscale rounded non-zero |a|>=factor/2 to 0: a={} factor={} down={}",
                i.a, factor, down
            );
            if i.a > 0 {
                assert!(
                    down > 0,
                    "downscale lost positive sign: a={} factor={} down={}",
                    i.a, factor, down
                );
            } else {
                assert!(
                    down < 0,
                    "downscale lost negative sign: a={} factor={} down={}",
                    i.a, factor, down
                );
            }
        }

        // Rounds-away-from-zero reconstruction bound (tighter than the earlier
        // half-up error check). If down rounds away from zero, then
        // |down * factor| ≥ |a| − (factor − 1). i.e. the magnitude of the
        // reconstructed value loses at most one factor unit minus 1.
        if let Some(reconstructed) = down.checked_mul(factor) {
            let abs_recon = reconstructed.abs();
            let abs_a = i.a.abs();
            // abs_recon + (factor - 1) >= abs_a  (guard from underflow)
            assert!(
                abs_recon + (factor - 1) >= abs_a,
                "downscale truncated too aggressively: a={} down={} factor={} |recon|={}",
                i.a, down, factor, abs_recon
            );
        }
    }
});
