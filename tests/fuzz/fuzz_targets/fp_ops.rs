
#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, WAD};
use common::math::fp::{Bps, Ray, Wad};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::Env;

const MAX_MAG: i128 = 10_000_000_000_000_000_000;

#[derive(Debug, Arbitrary)]
struct In {

    a_raw: u64,
    b_raw: u64,

    bps: u16,

    decimals: u8,
    token_amount: u64,
}

fn magnitude(raw: u64) -> i128 {
    (raw as i128) % MAX_MAG
}

fuzz_target!(|i: In| {
    let env = Env::default();

    let a = magnitude(i.a_raw);
    let b = magnitude(i.b_raw);
    let ray_a = Ray::from(a);
    let ray_b = Ray::from(b);
    let wad_a = Wad::from(a);
    let wad_b = Wad::from(b);
    let bps = Bps::from(i.bps as i128);
    let decimals = (i.decimals % 28) as u32;

    assert_eq!(
        ray_a.checked_add(&env, ray_b).checked_sub(&env, ray_b),
        ray_a,
        "Ray add/sub roundtrip"
    );
    assert_eq!(
        wad_a.checked_add(&env, wad_b).checked_sub(&env, wad_b),
        wad_a,
        "Wad add/sub roundtrip"
    );
    let bps_a = Bps::from(i.bps as i128);
    let bps_b = Bps::from((i.b_raw as i128) % (BPS * 2));
    assert_eq!(
        bps_a.checked_add(&env, bps_b).checked_sub(&env, bps_b),
        bps_a,
        "Bps add/sub roundtrip"
    );

    let ray_one_as_wad = Ray::ONE.to_wad();
    assert_eq!(
        ray_one_as_wad.raw(),
        WAD,
        "Ray::ONE.to_wad() != Wad::ONE ({})",
        ray_one_as_wad.raw()
    );

    let ray_small = Ray::from(a / 2);
    let ray_big = Ray::from(a);
    assert!(
        ray_big.to_wad().raw() + 1 >= ray_small.to_wad().raw(),
        "Ray::to_wad not monotonic"
    );

    if a <= 10i128.pow(18) && decimals <= 18 {
        let asset = ray_a.to_asset(decimals);
        let back = Ray::from_asset(asset, decimals);
        let err = (back.raw() - ray_a.raw()).abs();

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
    }

    let ident = wad_a.mul(&env, Wad::ONE);
    let ident_err = (ident.raw() - wad_a.raw()).abs();
    assert!(
        ident_err <= 1,
        "Wad mul near-identity: {} * 1 = {} (err {})",
        wad_a.raw(),
        ident.raw(),
        ident_err
    );

    if a >= WAD && b >= WAD {
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

        if wad_a.raw() >= wad_b.raw() {
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

    assert_eq!(
        bps.apply_to(&env, 0),
        0,
        "Bps::apply_to(0) != 0 for bps={}",
        bps.raw()
    );

    if bps.raw() <= BPS && a <= 10i128.pow(24) {
        let scaled = bps.apply_to(&env, ray_a.raw());
        assert!(
            scaled <= ray_a.raw() + 1,
            "Bps::apply_to expansion: bps={} a={} -> {}",
            bps.raw(),
            ray_a.raw(),
            scaled
        );
    }

    let full_bps = Bps::from(BPS);
    assert_eq!(full_bps.to_wad(&env).raw(), WAD, "Bps(BPS).to_wad() != WAD");

    assert_eq!(Bps::from(0).to_wad(&env).raw(), 0, "Bps(0).to_wad() != 0");

    if bps.raw() <= BPS && a <= 10i128.pow(15) {
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
