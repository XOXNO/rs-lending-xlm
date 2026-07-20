//! `common::math::fp` type ops: Ray/Wad/Bps roundtrips, mul/div, token conversion.
#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, WAD};
use common::math::fp::{Bps, Ray, Wad};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::Env;

/// Signed operand cap (~10^19 raw). Domain must reach `|a|,|b| ≥ WAD` so the
/// mul/div roundtrip property is reachable.
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

    // checked_add / checked_sub roundtrips. Restrict to non-negative operands —
    // checked_sub panics on a negative result by design.
    if ray_a.raw() >= 0 && ray_b.raw() >= 0 {
        assert_eq!(
            ray_a.checked_add(&env, ray_b).checked_sub(&env, ray_b),
            ray_a,
            "Ray add/sub roundtrip"
        );
    }
    if wad_a.raw() >= 0 && wad_b.raw() >= 0 {
        assert_eq!(
            wad_a.checked_add(&env, wad_b).checked_sub(&env, wad_b),
            wad_a,
            "Wad add/sub roundtrip"
        );
    }
    let bps_a = Bps::from(i.bps as i128);
    let bps_b = Bps::from((i.b_raw as i128) % (BPS * 2));
    if bps_a.raw() >= 0 && bps_b.raw() >= 0 {
        assert_eq!(
            bps_a.checked_add(&env, bps_b).checked_sub(&env, bps_b),
            bps_a,
            "Bps add/sub roundtrip"
        );
    }

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

    // `a * 1 ≈ a` within 1 ulp. Not exact for negative `a`: half-up rounds
    // toward +∞ (e.g. -k.5 → -k), matching protocol tolerance.
    let ident = wad_a.mul(&env, Wad::ONE);
    let ident_err = (ident.raw() - wad_a.raw()).abs();
    assert!(
        ident_err <= 1,
        "Wad mul near-identity: {} * 1 = {} (err {})",
        wad_a.raw(),
        ident.raw(),
        ident_err
    );

    // `mul(a,b).div(b) == a` within 2 ulp only when |a|,|b| ≥ WAD; below
    // that, `a*b/WAD` truncates so `* WAD / b` cannot recover `a`.
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

        // div_floor ≤ div (floor ≤ half-up); same |a|,|b| ≥ WAD domain.
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

    // ≤18 decimals: exact token roundtrip. Above WAD precision: half-up of
    // one WAD-sized token unit.
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
