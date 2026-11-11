#![no_main]
//! Pure-function fuzzing of the LP fair pricing (constant-product and
//! StableSwap) and the integer square root behind it. Each result is checked
//! against an independent bound or re-derivation.

use arbitrary::Arbitrary;
use common::constants::WAD;
use common::oracle::lp::{fair_lp_price_wad, isqrt_of_product, LpLeg, LpSupply};
use common::oracle::lp_stable::fair_stable_lp_price_wad;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{Env, U256};
use stellar_fuzz::edge_biased_amount;

#[derive(Debug, Arbitrary)]
struct LegIn {
    reserve: u64,
    reserve_mode: u8,
    decimals: u8,
    price: u64,
    price_mode: u8,
}

#[derive(Debug, Arbitrary)]
struct In {
    a: LegIn,
    b: LegIn,
    shares: u64,
    shares_mode: u8,
    share_decimals: u8,
    amp_lo: u32,
    amp_step: u32,
    sqrt_a: u128,
    sqrt_b: u128,
}

fn pick(raw: u64, mode: u8, hi: i128) -> i128 {
    match mode % 10 {
        0 => 0,
        1 => -1,
        2 => i128::MAX,
        3 => hi,
        _ => edge_biased_amount(raw, 1, hi),
    }
}

fn leg(l: &LegIn) -> LpLeg {
    LpLeg {
        reserve: pick(l.reserve, l.reserve_mode, 10i128.pow(30)),
        decimals: u32::from(l.decimals) % 20,
        price_wad: pick(l.price, l.price_mode, 1_000_000_000 * WAD),
    }
}

fn to_wad(amount: i128, decimals: u32) -> Option<i128> {
    amount.checked_mul(10i128.checked_pow(18u32.checked_sub(decimals)?)?)
}

fn value_wad(l: &LpLeg) -> Option<i128> {
    let scale = 10i128.checked_pow(l.decimals)?;
    let product = l.reserve.checked_mul(l.price_wad)?;
    Some(product.checked_add(scale / 2)? / scale)
}

/// `actual` against a float bound, with relative slack for float error and a few
/// units for integer rounding.
fn within(actual: i128, bound: f64, upper: bool) -> bool {
    let a = actual as f64;
    let slack = bound.abs() * 1e-9 + 4.0;
    if upper {
        a <= bound + slack
    } else {
        a >= bound - slack
    }
}

fn check_stable(env: &Env, i: &In, a: &LpLeg, b: &LpLeg, supply: &LpSupply, any_nonpositive: bool) {
    let amp1 = 1 + u128::from(i.amp_lo % 1_000_000);
    let amp2 = (amp1 + u128::from(i.amp_step % 1_000_000)).min(1_000_000);
    let st1 = fair_stable_lp_price_wad(env, a, b, supply, amp1);
    let st2 = fair_stable_lp_price_wad(env, a, b, supply, amp2);
    let st_swapped = fair_stable_lp_price_wad(env, b, a, supply, amp1);
    if any_nonpositive {
        assert!(
            st1.is_err(),
            "non-positive input priced as stable LP: {st1:?}"
        );
    }
    assert!(
        fair_stable_lp_price_wad(env, a, b, supply, 0).is_err(),
        "amp 0 accepted"
    );
    assert!(
        fair_stable_lp_price_wad(env, a, b, supply, 1_000_001).is_err(),
        "amp above MAX_AMP accepted"
    );
    // The solver orders legs by reserve, so both orders run the same arithmetic.
    assert_eq!(
        st1, st_swapped,
        "stable LP price depends on leg order: a={a:?} b={b:?} supply={supply:?} amp={amp1}"
    );
    if let Ok(p) = st1 {
        // D lies between 2*sqrt(xa*xb) and xa + xb.
        if let (Some(xa), Some(xb), Some(shares_wad)) = (
            to_wad(a.reserve, a.decimals),
            to_wad(b.reserve, b.decimals),
            to_wad(supply.total_shares, supply.decimals),
        ) {
            let per_share = a.price_wad.min(b.price_wad) as f64 / shares_wad as f64;
            let upper = (xa as f64 + xb as f64) * per_share;
            let lower = 2.0 * (xa as f64).sqrt() * (xb as f64).sqrt() * per_share;
            assert!(
                within(p, upper, true),
                "stable LP price {p} above arithmetic bound {upper}"
            );
            assert!(
                within(p, lower, false),
                "stable LP price {p} below geometric bound {lower}"
            );
        }
        if let Ok(p2) = st2 {
            let tol = (p / 100_000).max(4);
            assert!(
                p2 >= p.saturating_sub(tol),
                "stable LP price fell as amp rose: amp {amp1}->{amp2}, {p}->{p2}"
            );
        }
    }
}

fuzz_target!(|i: In| {
    let env = Env::default();

    // isqrt_of_product is the exact floor square root of the 256-bit product.
    let n = U256::from_u128(&env, i.sqrt_a).mul(&U256::from_u128(&env, i.sqrt_b));
    let r = isqrt_of_product(&env, i.sqrt_a, i.sqrt_b);
    assert!(r.mul(&r) <= n, "isqrt overshoots");
    let r1 = r.add(&U256::from_u32(&env, 1));
    // r + 1 >= 2^128 squares past every 256-bit product, and past U256 itself.
    if r1.to_u128().is_some() {
        assert!(r1.mul(&r1) > n, "isqrt undershoots");
    }

    let a = leg(&i.a);
    let b = leg(&i.b);
    let supply = LpSupply {
        total_shares: pick(i.shares, i.shares_mode, 10i128.pow(30)),
        decimals: u32::from(i.share_decimals) % 20,
    };
    let any_nonpositive = a.reserve <= 0
        || b.reserve <= 0
        || a.price_wad <= 0
        || b.price_wad <= 0
        || supply.total_shares <= 0;

    let cp = fair_lp_price_wad(&env, &a, &b, &supply);
    let cp_swapped = fair_lp_price_wad(&env, &b, &a, &supply);
    assert_eq!(
        cp, cp_swapped,
        "constant-product LP price depends on leg order"
    );
    if any_nonpositive {
        assert!(cp.is_err(), "non-positive input priced: {cp:?}");
    }
    if let Ok(p) = cp {
        assert!(p >= 0, "negative LP price");
        // 2*sqrt(va*vb) <= va + vb, so the fair price never exceeds the naive book value.
        if let (Some(va), Some(vb), Some(shares_wad)) = (
            value_wad(&a),
            value_wad(&b),
            to_wad(supply.total_shares, supply.decimals),
        ) {
            let book = (va as f64 + vb as f64) * WAD as f64 / shares_wad as f64;
            assert!(
                within(p, book, true),
                "LP price {p} exceeds book value {book}"
            );
            if va > 0 && vb > 0 {
                let floor = book * (va.min(vb) as f64 / va.max(vb) as f64);
                assert!(
                    within(p, floor, false),
                    "LP price {p} falls below the min/max book bound {floor}"
                );
            }
        }
    }

    check_stable(&env, &i, &a, &b, &supply, any_nonpositive);
});
