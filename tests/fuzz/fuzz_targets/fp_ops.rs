//! `common::math::fp` type ops: Ray/Wad/Bps roundtrips, mul/div, token conversion.
#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, WAD};
use common::math::fp::{Bps, Ray, Wad};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::Env;

/// Keeps signed operands in the realistic low-double-digit WAD range. The
/// previous exclusive 1-WAD cap made the `|a| >= WAD && |b| >= WAD`
/// multiplication/division property unreachable.
const MAX_MAG: i128 = 10_000_000_000_000_000_000; // 10^19

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
    // Amount for token conversion tests.
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
    let ray_a = Ray::from(a);
    let ray_b = Ray::from(b);
    let wad_a = Wad::from(a);
    let wad_b = Wad::from(b);
    let bps = Bps::from(i.bps as i128);
    let decimals = (i.decimals % 28) as u32; // 0..=27

    // Add / Sub roundtrips. Restrict to non-negative operands — Sub panics
    // on a negative result by design.
    if ray_a.raw() >= 0 && ray_b.raw() >= 0 {
        assert_eq!((ray_a + ray_b) - ray_b, ray_a, "Ray add/sub roundtrip");
    }
    if wad_a.raw() >= 0 && wad_b.raw() >= 0 {
        assert_eq!((wad_a + wad_b) - wad_b, wad_a, "Wad add/sub roundtrip");
    }
    let bps_a = Bps::from(i.bps as i128);
    let bps_b = Bps::from((i.b_raw as i128) % (BPS * 2));
    if bps_a.raw() >= 0 && bps_b.raw() >= 0 {
        assert_eq!((bps_a + bps_b) - bps_b, bps_a, "Bps add/sub roundtrip");
    }

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
    let ray_small = Ray::from(a.abs() / 2);
    let ray_big = Ray::from(a.abs());
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
    // Domain: the identity `mul(a,b).div(b) == a` holds within 2 ulp
    // when both |a| ≥ 1.0 Wad (= WAD raw) and |b| ≥ 1.0 Wad. Below 1.0,
    // the intermediate `a*b/WAD` truncates so aggressively that subsequent
    // `* WAD / b` cannot recover `a`. Smaller token-unit conversions are
    // covered by the precision-aware roundtrip below.
    if a.abs() >= WAD && b.abs() >= WAD {
        let prod = wad_a.mul(&env, wad_b);
        let roundtrip = prod.div(&env, wad_b);
        let err = (roundtrip.raw() - wad_a.raw()).abs();
        assert!(
            err <= 2,
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
    // At <=18 decimals the conversion roundtrip is exact. Above WAD
    // precision it quantizes once, within half one WAD-sized token unit.
    let token_amount = i.token_amount as i128;
    let w = Wad::from_token(token_amount, decimals);
    let back = w.to_token(decimals);
    if decimals <= 18 {
        assert_eq!(
            back, token_amount,
            "Wad token roundtrip at decimals={decimals}"
        );
    } else {
        let factor = 10i128.pow(decimals - 18);
        assert!(
            (back - token_amount).abs() <= factor / 2 + 1,
            "Wad token roundtrip exceeded half-up bound: amount={} back={} decimals={}",
            token_amount,
            back,
            decimals
        );
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
    let full_bps = Bps::from(BPS);
    assert_eq!(full_bps.to_wad(&env).raw(), WAD, "Bps(BPS).to_wad() != WAD");
    // Zero bps -> zero Wad.
    assert_eq!(Bps::from(0).to_wad(&env).raw(), 0, "Bps(0).to_wad() != 0");

    // apply_to_wad: apply_to_wad(x) should equal Wad::from(apply_to(x.raw()))
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
