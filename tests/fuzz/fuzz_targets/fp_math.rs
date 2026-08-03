
#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, RAY, WAD};
use common::math::fp_core::{div_by_int_half_up, mul_div_half_up, rescale_half_up};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::Env;

const MAX_OP: i128 = 10i128.pow(27);

const MAX_A: i128 = 10i128.pow(30);

#[derive(Debug, Arbitrary)]
struct In {
    kind: u8,
    a: i128,
    b: i128,
    choice: u8,
    extra: u8,
}

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

    let per_operand_cap = match d {
        RAY => 10i128.pow(27),
        WAD => 10i128.pow(27),
        _ => 10i128.pow(20),
    };
    let a = clamp_nonneg(i.a).min(per_operand_cap);
    let b = clamp_nonneg(i.b).min(per_operand_cap);

    let env = Env::default();
    let id = mul_div_half_up(&env, a, d, d);
    assert_eq!(id, a, "identity violated: {}*{}/{} != {}", a, d, d, a);

    let env2 = Env::default();
    let r1 = mul_div_half_up(&env2, a, b, d);
    let env3 = Env::default();
    let r2 = mul_div_half_up(&env3, b, a, d);
    assert_eq!(
        r1, r2,
        "commutativity: {}*{}/{} != {}*{}/{}",
        a, b, d, b, a, d
    );

    let env4 = Env::default();
    assert_eq!(mul_div_half_up(&env4, 0, b, d), 0);
    let env5 = Env::default();
    assert_eq!(mul_div_half_up(&env5, a, 0, d), 0);

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

    let a = i.a % (MAX_A + 1);
    let b = (i.b % MAX_A).saturating_abs() + 1;

    let r = div_by_int_half_up(a, b);

    if let Some(rb) = r.checked_mul(b) {
        let err = (rb - a).abs();
        assert!(
            err <= b / 2 + 1,
            "div_by_int error bound: a={} b={} r={} rb={} err={}",
            a,
            b,
            r,
            rb,
            err
        );
    }

    if a.abs() >= b {
        if a > 0 {
            assert!(r > 0, "sign lost for positive: {} / {} = {}", a, b, r);
        } else if a < 0 {
            assert!(r < 0, "sign lost for negative: {} / {} = {}", a, b, r);
        }
    }

    const F64_EXACT_MAX: i128 = 1i128 << 53;
    if a.abs() < F64_EXACT_MAX && b < F64_EXACT_MAX {
        let q = a as f64 / b as f64;
        let expected = if q >= 0.0 {
            (q + 0.5).floor() as i128
        } else {
            (q - 0.5).ceil() as i128
        };
        let diff = (r - expected).abs();
        assert!(
            diff <= 1,
            "div_by_int differs from f64 reference: a={} b={} r={} expected={} diff={}",
            a,
            b,
            r,
            expected,
            diff
        );
    }
}

fn fuzz_rescale(i: &In) {

    let from = (i.choice % 28) as u32;
    let to = (i.extra % 28) as u32;

    let a = i.a % (MAX_A + 1);

    if from == to {
        assert_eq!(rescale_half_up(a, from, to), a);
        return;
    }

    if to > from {
        let diff = to - from;
        let factor: i128 = 10i128.pow(diff);

        let bound = (i128::MAX / 2) / factor;
        let bounded = a % (bound + 1);
        let up = rescale_half_up(bounded, from, to);
        let back = rescale_half_up(up, to, from);
        assert_eq!(
            back, bounded,
            "upscale roundtrip lost data: a={} up={} back={}",
            bounded, up, back
        );
        if bounded > 0 {
            assert!(
                up > 0,
                "upscale lost positive sign: a={} -> {}",
                bounded,
                up
            );
        } else if bounded < 0 {
            assert!(
                up < 0,
                "upscale lost negative sign: a={} -> {}",
                bounded,
                up
            );
        }
    } else {
        let diff = from - to;
        let factor: i128 = 10i128.pow(diff);
        let down = rescale_half_up(a, from, to);
        if let Some(reconstructed) = down.checked_mul(factor) {
            let err = (reconstructed - a).abs();
            assert!(
                err <= factor / 2 + 1,
                "downscale exceeds half-up bound: a={} down={} recon={} err={} factor={}",
                a,
                down,
                reconstructed,
                err,
                factor
            );
        }

        if a.abs() >= factor / 2 && a != 0 {
            assert!(
                down != 0,
                "downscale rounded non-zero |a|>=factor/2 to 0: a={} factor={} down={}",
                a,
                factor,
                down
            );
            if a > 0 {
                assert!(
                    down > 0,
                    "downscale lost positive sign: a={} factor={} down={}",
                    a,
                    factor,
                    down
                );
            } else {
                assert!(
                    down < 0,
                    "downscale lost negative sign: a={} factor={} down={}",
                    a,
                    factor,
                    down
                );
            }
        }

        if let Some(reconstructed) = down.checked_mul(factor) {
            let abs_recon = reconstructed.abs();
            let abs_a = a.abs();
            assert!(
                abs_recon + (factor - 1) >= abs_a,
                "downscale truncated too aggressively: a={} down={} factor={} |recon|={}",
                a,
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
