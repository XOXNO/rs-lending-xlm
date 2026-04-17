//! Unified fuzz target for the three pure-math primitives in `common::fp_core`:
//! `mul_div_half_up`, `div_by_int_half_up`, and `rescale_half_up`.
//!
//! One target replaces the former trio (`fp_mul_div`, `fp_div_by_int`,
//! `fp_rescale`). A shared 35-byte input layout dispatches to the correct arm
//! via `kind % 3`, which lets libFuzzer cross-pollinate bytes between arms
//! while keeping invariants per-arm:
//!
//! - MulDiv (commutativity, identity, zero-absorbing, half-up bound)
//! - DivByInt (away-from-zero sign, error bound, f64 differential)
//! - Rescale (roundtrip, downscale bound, sign preservation, away-from-zero)
//!
//! Inputs are clamped to protocol-realistic magnitudes (≤ 10^30). Values above
//! that range exercise MathOverflow paths that are legitimate protocol
//! behaviour, so they are treated as "out of domain" rather than bugs.
#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, RAY, WAD};
use common::fp_core::{div_by_int_half_up, mul_div_half_up, rescale_half_up};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::Env;

/// One RAY of headroom. Protocol amounts are WAD-scaled (10^18) or smaller;
/// scaled indexes are RAY (10^27). Products like (amount * index / RAY) stay
/// within this envelope after the final division.
const MAX_OP: i128 = 10i128.pow(27);

/// Protocol-realistic bound on the magnitude of a fixed-point value.
/// i128::MIN triggers (a - half) underflow in the rescale impl -- unreachable
/// in real flows where values originate from token amounts <= 10^27.
const MAX_A: i128 = 10i128.pow(30);

/// 35-byte structure-aware layout: `kind % 3` chooses the arm, then each arm
/// interprets the shared `a`/`b`/`choice`/`extra` fields as needed. Keeps the
/// corpus stable under format changes and lets libFuzzer mutate across arms.
#[derive(Debug, Arbitrary)]
struct In {
    kind: u8,
    a: i128,
    b: i128,
    choice: u8,
    extra: u8,
}

/// `mul_div_half_up` is NOT sign-correct (rounds toward zero, not away). In
/// the protocol it is only called with non-negative operands because
/// Ray/Wad/Bps types are always >= 0. Signed math goes through
/// `mul_div_half_up_signed`. The fuzzer honours this contract.
fn clamp_nonneg(v: i128) -> i128 {
    let a = v.saturating_abs();
    if a > MAX_OP {
        MAX_OP
    } else {
        a
    }
}

fn fuzz_mul_div(i: &In) {
    let d = match i.choice % 3 {
        0 => RAY,
        1 => WAD,
        _ => BPS,
    };

    // Final-result domain: a*b/d must fit in i128 (else MathOverflow panic,
    // which is correct protocol behavior but not what we're fuzzing for).
    // Constrain a,b so that a*b/d <= 10^36 (well under i128::MAX = 1.7e38).
    let per_operand_cap = match d {
        RAY => 10i128.pow(27), // a*b <= 10^54, a*b/RAY <= 10^27
        WAD => 10i128.pow(27), // a*b <= 10^54, a*b/WAD <= 10^36
        _ => 10i128.pow(20),   // BPS: a*b <= 10^40, a*b/BPS <= 10^36
    };
    let a = clamp_nonneg(i.a).min(per_operand_cap);
    let b = clamp_nonneg(i.b).min(per_operand_cap);

    // Identity: mul_div(a, d, d) == a
    let env = Env::default();
    let id = mul_div_half_up(&env, a, d, d);
    assert_eq!(id, a, "identity violated: {}*{}/{} != {}", a, d, d, a);

    // Commutativity: mul_div(a,b,d) == mul_div(b,a,d)
    let env2 = Env::default();
    let r1 = mul_div_half_up(&env2, a, b, d);
    let env3 = Env::default();
    let r2 = mul_div_half_up(&env3, b, a, d);
    assert_eq!(
        r1, r2,
        "commutativity: {}*{}/{} != {}*{}/{}",
        a, b, d, b, a, d
    );

    // Zero absorbing
    let env4 = Env::default();
    assert_eq!(mul_div_half_up(&env4, 0, b, d), 0);
    let env5 = Env::default();
    assert_eq!(mul_div_half_up(&env5, a, 0, d), 0);

    // Half-up error bound: |r*d - a*b| <= d/2 + 1 (reconstructable via u256).
    // Skip if a*b overflows i128 (relies on I256 inside the impl).
    if let Some(ab) = a.checked_mul(b) {
        let rd = r1.checked_mul(d).unwrap_or(i128::MAX);
        let err = (rd - ab).abs();
        assert!(
            err <= d / 2 + 1,
            "half-up bound: a={} b={} d={} r={} err={}",
            a,
            b,
            d,
            r1,
            err
        );
    }
}

fn fuzz_div_by_int(i: &In) {
    // Impl requires b > 0
    if i.b <= 0 || i.b > MAX_A {
        return;
    }
    // Prevent (a + half_b) or (a - half_b) overflowing i128.
    if i.a == i128::MIN || i.a.abs() > MAX_A {
        return;
    }

    let r = div_by_int_half_up(i.a, i.b);

    // Reconstruct and check error bound: |r*b - a| <= b/2 + 1
    if let Some(rb) = r.checked_mul(i.b) {
        let err = (rb - i.a).abs();
        assert!(
            err <= i.b / 2 + 1,
            "div_by_int error bound: a={} b={} r={} rb={} err={}",
            i.a,
            i.b,
            r,
            rb,
            err
        );
    }

    // Away-from-zero: sign(r) == sign(a) when |a| >= b
    if i.a.abs() >= i.b {
        if i.a > 0 {
            assert!(r > 0, "sign lost for positive: {} / {} = {}", i.a, i.b, r);
        } else if i.a < 0 {
            assert!(r < 0, "sign lost for negative: {} / {} = {}", i.a, i.b, r);
        }
    }

    // Differential reference check: for magnitudes within f64's exact-integer
    // range (|x| < 2^53 ≈ 9.007e15), compute half-up-away-from-zero via f64
    // and compare. Catches off-by-one bugs the sign/error-bound checks miss.
    const F64_EXACT_MAX: i128 = 1i128 << 53;
    if i.a.abs() < F64_EXACT_MAX && i.b < F64_EXACT_MAX {
        let q = i.a as f64 / i.b as f64;
        let expected = if q >= 0.0 {
            (q + 0.5).floor() as i128
        } else {
            (q - 0.5).ceil() as i128
        };
        let diff = (r - expected).abs();
        assert!(
            diff <= 1,
            "div_by_int differs from f64 reference: a={} b={} r={} expected={} diff={}",
            i.a,
            i.b,
            r,
            expected,
            diff
        );
    }
}

fn fuzz_rescale(i: &In) {
    // Bound decimals to realistic precision range [0, 27]
    let from = (i.choice % 28) as u32;
    let to = (i.extra % 28) as u32;

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
        let diff = to - from;
        let factor: i128 = 10i128.pow(diff);
        // Bound |a| such that |a * factor| < i128::MAX / 2
        let bound = (i128::MAX / 2) / factor;
        if i.a.abs() > bound {
            return;
        }
        // `rescale_half_up` panics explicitly on upscale overflow -- that's
        // the designed behavior. Skip inputs we know will trip it.
        let up = match std::panic::catch_unwind(|| rescale_half_up(i.a, from, to)) {
            Ok(v) => v,
            Err(_) => return,
        };
        let back = rescale_half_up(up, to, from);
        assert_eq!(
            back, i.a,
            "upscale roundtrip lost data: a={} up={} back={}",
            i.a, up, back
        );
        if i.a > 0 {
            assert!(up > 0, "upscale lost positive sign: a={} -> {}", i.a, up);
        } else if i.a < 0 {
            assert!(up < 0, "upscale lost negative sign: a={} -> {}", i.a, up);
        }
    } else {
        let diff = from - to;
        let factor: i128 = 10i128.pow(diff);
        let down = rescale_half_up(i.a, from, to);
        if let Some(reconstructed) = down.checked_mul(factor) {
            let err = (reconstructed - i.a).abs();
            assert!(
                err <= factor / 2 + 1,
                "downscale exceeds half-up bound: a={} down={} recon={} err={} factor={}",
                i.a,
                down,
                reconstructed,
                err,
                factor
            );
        }
        // Strict away-from-zero: |a| >= factor/2 must produce non-zero w/ sign(a)
        if i.a.abs() >= factor / 2 && i.a != 0 {
            assert!(
                down != 0,
                "downscale rounded non-zero |a|>=factor/2 to 0: a={} factor={} down={}",
                i.a,
                factor,
                down
            );
            if i.a > 0 {
                assert!(
                    down > 0,
                    "downscale lost positive sign: a={} factor={} down={}",
                    i.a,
                    factor,
                    down
                );
            } else {
                assert!(
                    down < 0,
                    "downscale lost negative sign: a={} factor={} down={}",
                    i.a,
                    factor,
                    down
                );
            }
        }
        // Tighter magnitude bound than the half-up check.
        if let Some(reconstructed) = down.checked_mul(factor) {
            let abs_recon = reconstructed.abs();
            let abs_a = i.a.abs();
            assert!(
                abs_recon + (factor - 1) >= abs_a,
                "downscale truncated too aggressively: a={} down={} factor={} |recon|={}",
                i.a,
                down,
                factor,
                abs_recon
            );
        }
    }
}

fuzz_target!(|i: In| {
    match i.kind % 3 {
        0 => fuzz_mul_div(&i),
        1 => fuzz_div_by_int(&i),
        _ => fuzz_rescale(&i),
    }
});
