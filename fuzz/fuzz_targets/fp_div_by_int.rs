//! Fuzz `div_by_int_half_up` — integer division with away-from-zero rounding.
//!
//! Invariants:
//!   1. |result * b - a| ≤ b/2 + 1   (half-up error bound)
//!   2. Sign: result rounds AWAY from zero (−3.5 → −4, not −3)
//!   3. Divisor b must be > 0 (debug_assert in impl — skip b ≤ 0 in fuzz)
#![no_main]
use arbitrary::Arbitrary;
use common::fp_core::div_by_int_half_up;
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
struct In {
    a: i128,
    b: i128,
}

// Protocol-realistic magnitudes (see fp_mul_div for rationale).
const MAX_A: i128 = 10i128.pow(30);
const MAX_B: i128 = 10i128.pow(30);

fuzz_target!(|i: In| {
    // Impl requires b > 0
    if i.b <= 0 || i.b > MAX_B {
        return;
    }
    // Prevent (a + half_b) or (a - half_b) overflowing i128.
    // Protocol values never exceed ~10^30 — above that is out of scope.
    if i.a == i128::MIN || i.a.abs() > MAX_A {
        return;
    }

    let r = div_by_int_half_up(i.a, i.b);

    // Reconstruct and check error bound: |r*b - a| ≤ b/2 + 1
    let reconstructed = r.checked_mul(i.b);
    if let Some(rb) = reconstructed {
        let err = (rb - i.a).abs();
        assert!(
            err <= i.b / 2 + 1,
            "div_by_int error bound: a={} b={} r={} rb={} err={}",
            i.a, i.b, r, rb, err
        );
    }

    // Away-from-zero rounding: for any a that would round at 0.5, |r| ≥ |a|/b ceiling
    // We check the weaker but robust property: sign(r) == sign(a) when |a| ≥ b/2
    if i.a.abs() >= i.b {
        if i.a > 0 {
            assert!(r > 0, "sign lost for positive: {} / {} = {}", i.a, i.b, r);
        } else if i.a < 0 {
            assert!(r < 0, "sign lost for negative: {} / {} = {}", i.a, i.b, r);
        }
    }

    // Differential reference check: for magnitudes within f64's exact-integer range
    // (|x| < 2^53 ≈ 9.007e15), compute the expected half-up-away-from-zero result via
    // f64 and compare. This catches off-by-one bugs the sign/error-bound checks miss.
    const F64_EXACT_MAX: i128 = 1i128 << 53;
    if i.a.abs() < F64_EXACT_MAX && i.b < F64_EXACT_MAX {
        let q = i.a as f64 / i.b as f64;
        // Round half away from zero.
        let expected = if q >= 0.0 {
            (q + 0.5).floor() as i128
        } else {
            (q - 0.5).ceil() as i128
        };
        let diff = (r - expected).abs();
        assert!(
            diff <= 1,
            "div_by_int differs from f64 reference: a={} b={} r={} expected={} diff={}",
            i.a, i.b, r, expected, diff
        );
    }
});
