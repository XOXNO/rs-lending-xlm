#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, RAY, WAD};
use common::math::fp::{Bps, Ray, Wad};
use common::math::fp_core::{
    mul_div_ceil, mul_div_floor, mul_div_floor_saturating, mul_div_half_up,
};
use common::rates::{
    position_value, position_value_ceil, position_value_floor, resolve_net_settle, resolve_repay,
    resolve_withdrawal,
};
use libfuzzer_sys::fuzz_target;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::ToPrimitive;
use soroban_sdk::Env;

const MAX_MAG: i128 = 10_000_000_000_000_000_000;

#[derive(Debug, Arbitrary)]
struct In {
    a_raw: u64,
    b_raw: u64,

    bps: u16,

    decimals: u8,
    token_amount: u64,

    /// Directed-rounding and differential inputs.
    x_raw: u64,
    y_raw: u64,
    d_raw: u64,
    shift: u8,
    scaled_raw: u64,
    scaled_shift: u8,
    index_raw: u64,
    price_raw: u64,
    price_shift: u8,
    amount_raw: u64,
    debt_raw: u64,
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

    let ray_one_as_wad = Ray::ONE.to_wad(&env);
    assert_eq!(
        ray_one_as_wad.raw(),
        WAD,
        "Ray::ONE.to_wad(&env) != Wad::ONE ({})",
        ray_one_as_wad.raw()
    );

    let ray_small = Ray::from(a / 2);
    let ray_big = Ray::from(a);
    assert!(
        ray_big.to_wad(&env).raw() + 1 >= ray_small.to_wad(&env).raw(),
        "Ray::to_wad not monotonic"
    );

    if a <= 10i128.pow(18) && decimals <= 18 {
        let asset = ray_a.to_asset(&env, decimals);
        let back = Ray::from_asset(&env, asset, decimals);
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
    let w = Wad::from_token(&env, token_amount, decimals);
    let back = w.to_token(&env, decimals);
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
    assert_eq!(
        full_bps.to_wad(&env).raw(),
        WAD,
        "Bps(BPS).to_wad(&env) != WAD"
    );

    assert_eq!(
        Bps::from(0).to_wad(&env).raw(),
        0,
        "Bps(0).to_wad(&env) != 0"
    );

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

    directed_rounding(&env, &i);
    rational_differential(&env, &i);
});

fn big(v: i128) -> BigInt {
    BigInt::from(v)
}

fn ratio(n: BigInt, d: BigInt) -> BigRational {
    BigRational::new(n, d)
}

fn floor_i128(r: &BigRational) -> Option<i128> {
    r.floor().to_integer().to_i128()
}

fn ceil_i128(r: &BigRational) -> Option<i128> {
    r.ceil().to_integer().to_i128()
}

/// `(a + factor / 2) / factor` for non-negative `a`, the `rescale_half_up`
/// and `mul_div_half_up` rule, in exact arithmetic.
fn half_up_div(a: &BigInt, factor: &BigInt) -> BigInt {
    let half = factor / big(2);
    (a + half) / factor
}

fn ceil_div(a: &BigInt, factor: &BigInt) -> BigInt {
    (a + factor - big(1)) / factor
}

fn pow10(exp: u32) -> BigInt {
    big(10).pow(exp)
}

/// The four `mul_div` primitives, `mul_ratio_ceil`, `div_floor_saturating`
/// and `flash_loan_fee_on` against an exact quotient. Operands reach past
/// `i128::MAX` in the product, so the `I256` fallback is on the path.
fn directed_rounding(env: &Env, i: &In) {
    let x = magnitude(i.x_raw) * 10i128.pow((i.shift % 10) as u32);
    let y = magnitude(i.y_raw);
    let d = magnitude(i.d_raw).max(1);
    let exact = ratio(big(x) * big(y), big(d));
    let floor = floor_i128(&exact);
    let ceil = ceil_i128(&exact);
    match (floor, ceil) {
        (Some(fl), Some(ce)) => {
            assert_eq!(mul_div_floor(env, x, y, d), fl, "mul_div_floor {x} {y} {d}");
            assert_eq!(mul_div_ceil(env, x, y, d), ce, "mul_div_ceil {x} {y} {d}");
            assert_eq!(
                mul_div_floor_saturating(env, x, y, d),
                fl,
                "mul_div_floor_saturating {x} {y} {d}"
            );
            let half = half_up_div(&(big(x) * big(y)), &big(d))
                .to_i128()
                .expect("half-up lies between floor and ceil");
            assert_eq!(
                mul_div_half_up(env, x, y, d),
                half,
                "mul_div_half_up {x} {y} {d}"
            );
            assert!(fl <= half && half <= ce, "half-up outside [floor, ceil]");
            assert_eq!(
                Ray::from(x).mul_ratio_ceil(env, y, d).raw(),
                ce,
                "mul_ratio_ceil {x} {y} {d}"
            );
        }
        _ => {
            assert_eq!(
                mul_div_floor_saturating(env, x, y, d),
                i128::MAX,
                "saturating quotient past i128::MAX must clamp: {x} {y} {d}"
            );
        }
    }

    let wad_x = Wad::from(x);
    let wad_y = Wad::from(y.max(1));
    let exact_div = ratio(big(x) * big(WAD), big(y.max(1)));
    match floor_i128(&exact_div) {
        Some(fl) => assert_eq!(
            wad_x.div_floor_saturating(env, wad_y).raw(),
            fl,
            "Wad::div_floor_saturating {x} {y}"
        ),
        None => assert_eq!(
            wad_x.div_floor_saturating(env, wad_y).raw(),
            i128::MAX,
            "Wad::div_floor_saturating must clamp: {x} {y}"
        ),
    }

    let rate = Bps::from((i.bps as i128) % (BPS + 1));
    let amount = magnitude(i.amount_raw);
    let fee = half_up_div(&(big(amount) * big(rate.raw())), &big(BPS))
        .to_i128()
        .expect("fee fits");
    let expected = if rate.raw() > 0 && fee == 0 { 1 } else { fee };
    assert_eq!(
        rate.flash_loan_fee_on(env, amount),
        expected,
        "flash_loan_fee_on {amount} at {} bps",
        rate.raw()
    );
}

/// Exact mirrors of `position_value*`, `resolve_repay`, `resolve_withdrawal`
/// and `resolve_net_settle`, stage by stage in `BigInt`, plus the bracket
/// `floor <= exact <= ceil` that the directed rounding must respect.
fn rational_differential(env: &Env, i: &In) {
    // scaled up to 1e36 raw ray, index in [1x, ~11x], price up to 1e25 wad,
    // token decimals 0..=18: every intermediate stays inside i128.
    let scaled = magnitude(i.scaled_raw) * 10i128.pow((i.scaled_shift % 18) as u32);
    let index = RAY + magnitude(i.index_raw) * 1_000_000_000;
    let price = magnitude(i.price_raw) * 10i128.pow((i.price_shift % 7) as u32);
    let decimals = (i.decimals % 19) as u32;
    let ray = big(RAY);
    let wad = big(WAD);
    let ray_to_wad = pow10(9);

    // position_value: half-up at every stage.
    let stage1 = half_up_div(&(big(scaled) * big(index)), &ray);
    let stage2 = half_up_div(&stage1, &ray_to_wad);
    let expect_half = half_up_div(&(&stage2 * big(price)), &wad);
    let got_half = position_value(env, Ray::from(scaled), Ray::from(index), Wad::from(price)).raw();
    assert_eq!(
        big(got_half),
        expect_half,
        "position_value {scaled} {index} {price}"
    );

    // position_value_floor: floor at every stage.
    let f1 = (big(scaled) * big(index)) / &ray;
    let f2 = &f1 / &ray_to_wad;
    let expect_floor = (&f2 * big(price)) / &wad;
    let got_floor =
        position_value_floor(env, Ray::from(scaled), Ray::from(index), Wad::from(price)).raw();
    assert_eq!(
        big(got_floor),
        expect_floor,
        "position_value_floor {scaled} {index} {price}"
    );

    // position_value_ceil: ceiling at every stage.
    let c1 = ceil_div(&(big(scaled) * big(index)), &ray);
    let c2 = ceil_div(&c1, &ray_to_wad);
    let expect_ceil = ceil_div(&(&c2 * big(price)), &wad);
    let got_ceil =
        position_value_ceil(env, Ray::from(scaled), Ray::from(index), Wad::from(price)).raw();
    assert_eq!(
        big(got_ceil),
        expect_ceil,
        "position_value_ceil {scaled} {index} {price}"
    );

    // The bracket: floor <= exact <= ceil, and half-up between them.
    let exact_value = ratio(
        big(scaled) * big(index) * big(price),
        &ray * &ray_to_wad * &wad,
    );
    assert!(
        BigRational::from(big(got_floor)) <= exact_value,
        "position_value_floor above the exact value"
    );
    assert!(
        BigRational::from(big(got_ceil)) >= exact_value,
        "position_value_ceil below the exact value"
    );
    assert!(
        got_floor <= got_half && got_half <= got_ceil,
        "half-up outside [floor, ceil]"
    );

    // Token-unit conversions: amounts are exact in ray for decimals <= 27.
    let unit = pow10(27 - decimals);
    let exact_supply = ratio(big(scaled) * big(index), &ray * &unit);
    let supply_floor = floor_i128(&exact_supply).expect("supply fits");
    let supply_actual = half_up_div(&half_up_div(&(big(scaled) * big(index)), &ray), &unit)
        .to_i128()
        .expect("supply fits");
    let debt_scaled = magnitude(i.debt_raw) * 10i128.pow((i.scaled_shift % 18) as u32);
    let exact_debt = ratio(big(debt_scaled) * big(index), &ray * &unit);
    let debt_ceil = ceil_i128(&exact_debt).expect("debt fits");
    // amount * 10^(27 - decimals) must fit i128 for the scaling helpers.
    let amount = magnitude(i.amount_raw) % 10i128.pow(10 + decimals);
    let to_scaled = |amount: i128| ratio(big(amount) * &unit * &ray, big(index));

    // resolve_repay
    let (burned, refund) = resolve_repay(
        env,
        amount,
        Ray::from(debt_scaled),
        Ray::from(index),
        decimals,
    );
    if amount >= debt_ceil {
        assert_eq!(burned.raw(), debt_scaled, "resolve_repay full burn");
        assert_eq!(refund, amount - debt_ceil, "resolve_repay refund");
    } else {
        let expect = floor_i128(&to_scaled(amount)).expect("scaled fits");
        assert_eq!(burned.raw(), expect, "resolve_repay partial burn floors");
        assert_eq!(refund, 0, "resolve_repay partial refund");
    }

    // resolve_withdrawal
    let (burned, paid) =
        resolve_withdrawal(env, amount, Ray::from(scaled), Ray::from(index), decimals);
    if amount >= supply_actual {
        assert_eq!(burned.raw(), scaled, "resolve_withdrawal full burn");
        assert_eq!(paid, supply_floor, "resolve_withdrawal pays the floor");
    } else {
        let expect = ceil_i128(&to_scaled(amount)).expect("scaled fits");
        assert_eq!(
            burned.raw(),
            expect,
            "resolve_withdrawal partial burn ceils"
        );
        assert_eq!(paid, amount, "resolve_withdrawal partial pays the request");
    }
    assert!(
        BigRational::from(big(paid)) <= exact_supply.max(BigRational::from(big(amount))),
        "withdrawal paid more than the position or the request"
    );

    // resolve_net_settle
    let (burned_supply, burned_debt, settle) = resolve_net_settle(
        env,
        amount,
        Ray::from(scaled),
        Ray::from(debt_scaled),
        Ray::from(index),
        Ray::from(index),
        decimals,
    );
    let expect_settle = amount.min(supply_floor).min(debt_ceil);
    if expect_settle <= 0 {
        assert_eq!(
            (burned_supply.raw(), burned_debt.raw(), settle),
            (0, 0, 0),
            "net settle noop"
        );
    } else {
        assert_eq!(settle, expect_settle, "net settle amount");
        let expect_supply = if settle == supply_floor {
            scaled
        } else {
            ceil_i128(&to_scaled(settle)).expect("fits").min(scaled)
        };
        let expect_debt = if settle == debt_ceil {
            debt_scaled
        } else {
            floor_i128(&to_scaled(settle))
                .expect("fits")
                .min(debt_scaled)
        };
        assert_eq!(burned_supply.raw(), expect_supply, "net settle supply burn");
        assert_eq!(burned_debt.raw(), expect_debt, "net settle debt burn");
    }
}
