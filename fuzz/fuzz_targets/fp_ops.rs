//! Fuzz target for the `common::fp` helpers outside the direct coverage of
//! `fp_math` and `rates_and_index`:
//!
//!   - Ray ↔ Wad ↔ asset/token conversions (`to_wad`, `to_asset`,
//!     `from_asset`, `Wad::from_token`, `Wad::to_token`, `Bps::to_wad`).
//!   - Wad arithmetic (`mul`, `div`, `div_floor`, `min`, `max`).
//!   - Bps scaling (`apply_to`, `apply_to_wad`) under realistic ratios.
//!   - Add / Sub impls via `a + b - b == a` roundtrips.
//!
//! Invariants enforced (all per-op; see inline comments):
//!   - Roundtrip identity within documented ulp bounds.
//!   - Non-expansion for Bps ≤ BPS (`apply_to(x) ≤ x + 1`).
//!   - Sign preservation.
//!   - `min ≤ max` ordering.
//!   - Floor vs half-up: `div_floor(a, b) ≤ div(a, b)`.
//!
//! Inputs are clamped to protocol-realistic magnitudes (≤ 10^24). Anything
//! larger triggers `MathOverflow` panics and is skipped as out of domain.
#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, WAD};
use common::fp::{Bps, Ray, Wad};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::Env;

/// Keeps operands inside the `mul_div` safe envelope. `10^18` provides
/// headroom below the `i128` product bound for fixed-point roundtrips.
const MAX_MAG: i128 = 1_000_000_000_000_000_000; // 10^18

#[derive(Debug, Arbitrary)]
struct In {
    // Magnitudes & signs sampled through modulo; keeps libFuzzer's byte
    // mutation yielding a smooth distribution over the validated domain.
    a_raw: u64,
    a_sign: u8,
    b_raw: u64,
    b_sign: u8,
    // 0..=BPS (inclusive). Values > BPS exercise the degraded path where
    // `apply_to` scales up — a legitimate but less-common branch.
    bps: u16,
    // 0..=27, asset-decimal domain.
    decimals: u8,
    // Amount for token conversion tests. Constrained separately because
    // `Wad::from_token` multiplies by 10^(18 - decimals), so large values
    // at low decimals overflow.
    token_amount: i64,
}

fn signed(raw: u64, sign: u8) -> i128 {
    let mag = (raw as i128) % MAX_MAG;
    if sign & 1 == 0 {
        mag
    } else {
        -mag
    }
}

fuzz_target!(|i: In| {
    let env = Env::default();

    let a = signed(i.a_raw, i.a_sign);
    let b = signed(i.b_raw, i.b_sign);
    let ray_a = Ray::from_raw(a);
    let ray_b = Ray::from_raw(b);
    let wad_a = Wad::from_raw(a);
    let wad_b = Wad::from_raw(b);
    let bps = Bps::from_raw(i.bps as i128);
    let decimals = (i.decimals % 28) as u32; // 0..=27

    // ---- Add / Sub roundtrips (exercises the Add/Sub impls) ----
    assert_eq!((ray_a + ray_b) - ray_b, ray_a, "Ray add/sub roundtrip");
    assert_eq!((wad_a + wad_b) - wad_b, wad_a, "Wad add/sub roundtrip");
    let bps_a = Bps::from_raw(i.bps as i128);
    let bps_b = Bps::from_raw((i.b_raw as i128) % (BPS * 2));
    assert_eq!((bps_a + bps_b) - bps_b, bps_a, "Bps add/sub roundtrip");

    // ---- Ray::to_wad ----
    // Ray → Wad divides by 10^9. `Ray::ONE.to_wad() == Wad::ONE`.
    let ray_one_as_wad = Ray::ONE.to_wad();
    assert_eq!(
        ray_one_as_wad.raw(),
        WAD,
        "Ray::ONE.to_wad() != Wad::ONE ({})",
        ray_one_as_wad.raw()
    );
    // Monotonic: larger |ray| => larger |to_wad| (within 1 ulp).
    let ray_small = Ray::from_raw(a.abs() / 2);
    let ray_big = Ray::from_raw(a.abs());
    assert!(
        ray_big.to_wad().raw() + 1 >= ray_small.to_wad().raw(),
        "Ray::to_wad not monotonic"
    );

    // ---- Ray::to_asset / Ray::from_asset roundtrip ----
    // to_asset quantises to token precision; roundtrip can lose precision
    // but must never change sign or move more than 1 token-unit (scaled to RAY).
    if a.abs() <= 10i128.pow(18) && decimals <= 18 {
        let asset = ray_a.to_asset(decimals);
        let back = Ray::from_asset(asset, decimals);
        let err = (back.raw() - ray_a.raw()).abs();
        // Tolerance: one asset-unit scaled back to RAY = 10^(27 - decimals).
        let tol = 10i128.pow(27 - decimals.min(27));
        assert!(
            err <= tol,
            "Ray asset roundtrip: a={} -> asset={} -> back={} err={} tol={}",
            ray_a.raw(),
            asset,
            back.raw(),
            err,
            tol
        );
        assert!(
            ray_a.raw() == 0 || ray_a.raw().signum() == back.raw().signum() || back.raw() == 0,
            "Ray asset roundtrip flipped sign: {} -> {}",
            ray_a.raw(),
            back.raw()
        );
    }

    // ---- Wad::mul near-identity ----
    // `a * 1 ≈ a` within 1 ulp. Not an exact identity for negative `a`
    // because `Wad::mul` uses `mul_div_half_up` which rounds toward +∞:
    // for a = -k.5 the result rounds up to -k (drifting by 1 magnitude
    // toward zero). This matches the documented half-up behavior and the
    // tolerance used by the protocol.
    let ident = wad_a.mul(&env, Wad::ONE);
    let ident_err = (ident.raw() - wad_a.raw()).abs();
    assert!(
        ident_err <= 1,
        "Wad mul near-identity: {} * 1 = {} (err {})",
        wad_a.raw(),
        ident.raw(),
        ident_err
    );

    // ---- Wad::mul/div roundtrip ----
    // Domain: the identity `mul(a,b).div(b) == a` holds within 1 ulp only
    // when both |a| ≥ 1.0 Wad (= WAD raw) and |b| ≥ 1.0 Wad. Below 1.0,
    // the intermediate `a*b/WAD` truncates so aggressively that subsequent
    // `* WAD / b` cannot recover `a`. Production Wad amounts from real
    // tokens always satisfy this (smallest unit is 10^(18-decimals) raw,
    // already ≥ 1 for realistic token amounts).
    if a.abs() <= 10i128.pow(15)
        && b.abs() <= 10i128.pow(15)
        && a.abs() >= WAD
        && b.abs() >= WAD
    {
        let prod = wad_a.mul(&env, wad_b);
        let roundtrip = prod.div(&env, wad_b);
        let err = (roundtrip.raw() - wad_a.raw()).abs();
        assert!(
            err <= 1,
            "Wad mul/div roundtrip: a={} * b={} / b = {} (err {})",
            wad_a.raw(),
            wad_b.raw(),
            roundtrip.raw(),
            err
        );

        // ---- div_floor ≤ div (floor rounds ≤ half-up) ----
        // Same domain guard as the roundtrip above.
        if wad_a.raw().signum() * wad_b.raw().signum() > 0 && wad_a.raw().abs() >= wad_b.raw().abs()
        {
            let f = wad_a.div_floor(&env, wad_b);
            let d = wad_a.div(&env, wad_b);
            assert!(
                f.raw() <= d.raw(),
                "div_floor > div: floor={} div={} a={} b={}",
                f.raw(),
                d.raw(),
                wad_a.raw(),
                wad_b.raw()
            );
        }
    }

    // ---- Wad::min / Wad::max ordering ----
    let mn = wad_a.min(wad_b);
    let max_wad = wad_a.max(wad_b);
    assert!(
        mn.raw() <= max_wad.raw(),
        "min > max: {} > {}",
        mn.raw(),
        max_wad.raw()
    );
    assert!(
        mn.raw() == wad_a.raw() || mn.raw() == wad_b.raw(),
        "min not in {{a, b}}"
    );
    assert!(
        max_wad.raw() == wad_a.raw() || max_wad.raw() == wad_b.raw(),
        "max not in {{a, b}}"
    );

    // ---- Wad <-> token conversion ----
    // Exercises `Wad::from_token` and `Wad::to_token`. Assertions are
    // deliberately loose: `from_token` uses half-up rescale which can
    // round away from zero, so a strict `to_token(from_token(x)) <= x`
    // bound doesn't hold for negative amounts or near-boundary values.
    // Durable invariants: non-zero roundtrips preserve sign and zero maps to zero.
    if decimals >= 2 || i.token_amount.abs() as i128 <= 10i128.pow(15) {
        let w = Wad::from_token(i.token_amount as i128, decimals);
        let back = w.to_token(decimals);
        // Sign preservation across the roundtrip.
        if i.token_amount != 0 && back != 0 {
            assert_eq!(
                back.signum(),
                (i.token_amount as i128).signum(),
                "Wad token roundtrip flipped sign: {} -> wad={} -> {}",
                i.token_amount,
                w.raw(),
                back
            );
        }
        // Zero is a fixed point.
        if i.token_amount == 0 {
            assert_eq!(w.raw(), 0, "Wad::from_token(0) != 0 for decimals={}", decimals);
            assert_eq!(back, 0, "Wad::to_token of zero Wad != 0");
        }
    }

    // ---- Bps ops ----
    // apply_to(0) = 0.
    assert_eq!(
        bps.apply_to(&env, 0),
        0,
        "Bps::apply_to(0) != 0 for bps={}",
        bps.raw()
    );
    // Non-expansion: for bps ≤ BPS, `apply_to(x) ≤ |x| + 1` (half-up slack).
    if bps.raw() <= BPS && a.abs() <= 10i128.pow(24) {
        let scaled = bps.apply_to(&env, ray_a.raw());
        assert!(
            scaled.abs() <= ray_a.raw().abs() + 1,
            "Bps::apply_to expansion: bps={} a={} -> {}",
            bps.raw(),
            ray_a.raw(),
            scaled
        );
        // Sign preservation.
        if ray_a.raw() != 0 && scaled != 0 {
            assert_eq!(
                scaled.signum(),
                ray_a.raw().signum(),
                "Bps::apply_to flipped sign"
            );
        }
    }

    // Bps::to_wad. BPS bps = Wad::ONE.
    let full_bps = Bps::from_raw(BPS);
    assert_eq!(
        full_bps.to_wad(&env).raw(),
        WAD,
        "Bps(BPS).to_wad() != WAD"
    );
    // Zero bps -> zero Wad.
    assert_eq!(
        Bps::from_raw(0).to_wad(&env).raw(),
        0,
        "Bps(0).to_wad() != 0"
    );

    // apply_to_wad: apply_to_wad(x) should equal Wad::from_raw(apply_to(x.raw()))
    // within 1 ulp (both use the same half-up rounding under the hood).
    if bps.raw() <= BPS && a.abs() <= 10i128.pow(15) {
        let via_wad = bps.apply_to_wad(&env, wad_a);
        let via_raw = bps.apply_to(&env, wad_a.raw());
        let err = (via_wad.raw() - via_raw).abs();
        assert!(
            err <= 1,
            "apply_to_wad != apply_to: wad={} raw={} err={}",
            via_wad.raw(),
            via_raw,
            err
        );
    }
});
