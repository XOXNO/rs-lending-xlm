//! RV rounding audit, lens R-5, round 1: bonus derivation and the HF-preserving cap.
//!
//! Integer mirror of `contracts/controller/src/positions/liquidation/curve.rs`
//! (`max_bonus_for_threshold` lines 192-202, `max_hf_preserving_bonus_bps` 85-94,
//! `calculate_linear_bonus_with_target` 60-81, `liquidation_at_target` 171-188,
//! `estimate_liquidation_amount` 102-138) and of the USD-weighted base bonus in
//! `math.rs` `get_account_bonus_params` (lines 570-607), built from the same
//! `fp_core` calls, checked against exact rationals and, in the last test, against
//! the live controller through `get_liquidation_estimate`.
//!
//! Every assertion is the documented bound (`docs/reference/formulas.md`, "Bonus and
//! target repayment"):
//! * the threshold bound `floor(BPS*(BPS-t)/t)`, `t = clamp(ceil(p*BPS/WAD),1,BPS)`,
//!   is HF-neutral at the half-up `p` and never above the exact `BPS*(WAD-p)/p`;
//! * the HF-preserving cap `floor(HF*BPS/p) - BPS` exceeds `C/D - 1` by at most the
//!   raw rounding slack `C / (2*W*WAD - C)` (about 1e-18 relative), never by a BPS;
//! * at `C == D` the cap is -1 or 0 and the band quote is `(D, 0)`;
//! * the base bonus is within half a BPS per leg of the exact USD-weighted average
//!   (at most `POSITION_LIMIT_MAX / 2 = 2.5` BPS) and never above the threshold bound;
//! * the curve bonus is within one BPS of the exact ramp and inside `[base, max]`;
//! * the quoted seizure `quote * (1 + bonus)` never exceeds `C` beyond that slack plus
//!   half a raw WAD, and a solvent partial never lowers `C/D` beyond it.

use common::math::fp::{Ray, Wad};
use common::math::fp_core::{
    mul_div_ceil, mul_div_floor, mul_div_floor_saturating, mul_div_half_up,
};
use common::types::{AccountPositionRaw, ControllerKey, HubAssetKey, SeizeMode};
use controller::constants::{
    BAD_DEBT_USD_THRESHOLD, BPS, DEFAULT_HF_FOR_MAX_BONUS_WAD,
    DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS, DEFAULT_LIQUIDATION_TARGET_HF_WAD,
    MAX_LIQUIDATION_TARGET_HF_WAD, POSITION_LIMIT_MAX, WAD,
};
use controller::types::PriceKey;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, ToPrimitive, Zero};
use soroban_sdk::{Env, Map};
use test_harness::{
    hub_asset, AssetConfigPreset, LendingTest, MarketPreset, ALICE, DEFAULT_ASSET_CONFIG,
    DEFAULT_MARKET_PARAMS, LIQUIDATOR,
};

// ---------------------------------------------------------------------------
// Deterministic PRNG and rational helpers
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[lo, hi]`.
    fn range(&mut self, lo: i128, hi: i128) -> i128 {
        assert!(hi >= lo, "range: lo={lo} hi={hi}");
        let span = (hi - lo + 1) as u128;
        let r = (((self.next() as u128) << 64) | self.next() as u128) % span;
        lo + r as i128
    }

    /// Log-uniform over the decades of `[lo, hi]` (`lo > 0`).
    fn log_range(&mut self, lo: i128, hi: i128) -> i128 {
        let decades = (lo.ilog10() as i128, hi.ilog10() as i128);
        let e = self.range(decades.0, decades.1) as u32;
        let base = 10i128.pow(e);
        self.range(base, base * 10 - 1).clamp(lo, hi)
    }

    fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

fn big(v: i128) -> BigInt {
    BigInt::from(v)
}

fn rat(v: i128) -> BigRational {
    BigRational::from_integer(big(v))
}

fn frac(num: BigInt, den: BigInt) -> BigRational {
    BigRational::new(num, den)
}

// ---------------------------------------------------------------------------
// Integer mirror of curve.rs and the base-bonus average, same fp calls
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Snap {
    d: i128,
    c: i128,
    w: i128,
    p: i128,
    hf: i128,
}

#[derive(Clone, Copy, Debug)]
struct Curve {
    h: i128,
    k: i128,
    f: i128,
}

const DEFAULT_CURVE: Curve = Curve {
    h: DEFAULT_LIQUIDATION_TARGET_HF_WAD,
    k: DEFAULT_HF_FOR_MAX_BONUS_WAD,
    f: DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS as i128,
};

/// `risk/totals.rs` lines 203-207: `HF = floor(W / D)` saturating, `i128::MAX`
/// without debt. `liquidation/math.rs` lines 357-361: `p = half_up(W / C)`, zero
/// without collateral.
fn snap_of(env: &Env, w: i128, c: i128, d: i128) -> Snap {
    let hf = if d == 0 {
        i128::MAX
    } else {
        mul_div_floor_saturating(env, w, WAD, d)
    };
    let p = if c > 0 {
        mul_div_half_up(env, w, WAD, c)
    } else {
        0
    };
    Snap { d, c, w, p, hf }
}

/// `max_bonus_for_threshold`, curve.rs lines 192-202.
fn mirror_max_bonus(env: &Env, p: i128) -> i128 {
    if p <= 0 {
        return 0;
    }
    let t = mul_div_ceil(env, p, BPS, WAD).clamp(1, BPS);
    BPS * (BPS - t) / t
}

/// `max_hf_preserving_bonus_bps`, curve.rs lines 85-94.
fn mirror_cap(s: &Snap) -> Option<i128> {
    if s.p <= 0 || s.hf >= WAD {
        return None;
    }
    Some(s.hf * BPS / s.p - BPS)
}

/// `LiquidationCurve::bonus_scale` and `calculate_linear_bonus_with_target`,
/// curve.rs lines 46-81. Requires `base <= max` as production does.
fn mirror_curve(env: &Env, hf: i128, base: i128, max: i128, cv: &Curve) -> i128 {
    if hf >= cv.h {
        return base;
    }
    let scale = if cv.h <= cv.k {
        WAD
    } else {
        mul_div_half_up(env, cv.h - hf, WAD, cv.h - cv.k).min(WAD)
    };
    assert!(base <= max, "production checked_sub panics on base > max");
    let increment = mul_div_half_up(env, max - base, scale, WAD);
    let scaled = mul_div_half_up(env, increment, cv.f, BPS);
    base + scaled
}

/// `Bps::to_wad`, fp.rs line 285: `half_up(bps * WAD / BPS)`.
fn bps_to_wad(env: &Env, bps: i128) -> i128 {
    mul_div_half_up(env, bps, WAD, BPS)
}

/// `liquidation_at_target`, curve.rs lines 171-188.
fn mirror_at_target(env: &Env, s: &Snap, bonus: i128, h: i128) -> i128 {
    let one_plus = WAD + bps_to_wad(env, bonus);
    let d_max = mul_div_half_up(env, s.c, WAD, one_plus);
    let denom_term = mul_div_half_up(env, s.p, one_plus, WAD);
    let target_debt = mul_div_half_up(env, h, s.d, WAD);
    if h <= denom_term || target_debt <= s.w {
        return d_max.min(s.d);
    }
    let numerator = target_debt - s.w;
    let denominator = h - denom_term;
    mul_div_half_up(env, numerator, WAD, denominator)
        .min(d_max)
        .min(s.d)
}

/// `estimate_liquidation_amount`, curve.rs lines 102-138. Returns `(quote, bonus)`.
fn mirror_estimate(env: &Env, s: &Snap, base: i128, max: i128, cv: &Curve) -> (i128, i128) {
    let scaled = mirror_curve(env, s.hf, base, max, cv);
    let bonus = match mirror_cap(s) {
        None => scaled,
        Some(_) if s.c < s.d => {
            let one_plus_base = WAD + bps_to_wad(env, base);
            let backed = mul_div_floor(env, s.c, WAD, one_plus_base);
            return (backed.min(s.d), base);
        }
        Some(cap) if cap < base => return (s.d, cap.max(0)),
        Some(cap) => scaled.min(cap),
    };
    let ideal = mirror_at_target(env, s, bonus, cv.h);
    let remaining = s.d - ideal;
    if remaining > 0 && remaining < BAD_DEBT_USD_THRESHOLD {
        return (s.d, bonus);
    }
    (ideal, bonus)
}

/// `get_account_bonus_params`, math.rs lines 570-607: one half-up weight and one
/// half-up product per leg, summed, capped at `max`.
fn mirror_base(env: &Env, values: &[i128], bonuses: &[i128], c: i128, max: i128) -> i128 {
    if c == 0 {
        return 0;
    }
    let mut sum = 0i128;
    for (v, b) in values.iter().zip(bonuses) {
        let weight = mul_div_half_up(env, *v, WAD, c);
        sum += mul_div_half_up(env, weight, *b, WAD);
    }
    sum.min(max)
}

/// Exact USD-weighted average of the listing bonuses, in BPS.
fn exact_base(values: &[i128], bonuses: &[i128]) -> BigRational {
    let c: BigInt = values.iter().map(|v| big(*v)).sum();
    let mut sum = BigRational::zero();
    for (v, b) in values.iter().zip(bonuses) {
        sum += frac(big(*v) * big(*b), c.clone());
    }
    sum
}

/// Documented slack of the HF-preserving cap: with `A = W * WAD`,
/// `HF <= A / D` (floor) and `p >= A / C - 1/2` (half-up), so
/// `BPS + cap <= (C * BPS / D) * 2A / (2A - C)`. Asserts it and returns the
/// relative overshoot `(BPS + cap) * D / (C * BPS) - 1`.
fn assert_cap_within_slack(s: &Snap, cap: i128, label: &str) -> BigRational {
    let a2 = big(s.w) * big(WAD) * big(2);
    let denom = &a2 - big(s.c);
    assert!(denom > BigInt::zero(), "{label}: p > 0 implies 2A > C");
    let lhs = rat(BPS + cap) * rat(s.d);
    let rhs = frac(big(s.c) * big(BPS) * a2, denom);
    assert!(
        lhs <= rhs,
        "{label}: (BPS+cap)*D = {lhs} exceeds C*BPS*2A/(2A-C) = {rhs} for {s:?} cap={cap}"
    );
    lhs / (rat(s.c) * rat(BPS)) - BigRational::one()
}

fn f64_of(r: &BigRational) -> f64 {
    r.to_f64().unwrap_or(f64::NAN)
}

// ---------------------------------------------------------------------------
// S1: threshold bound `floor(BPS*(BPS-t)/t)`
// ---------------------------------------------------------------------------

#[test]
fn rv_round_r5_threshold_max_bonus_is_hf_neutral_and_never_above_exact() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    let mut rng = Rng(0x5eed_0001);
    let mut ps: Vec<i128> = (1..=50).collect();
    for k in 1..=BPS {
        let b = k * (WAD / BPS);
        ps.extend([b - 1, b, b + 1]);
    }
    for _ in 0..20_000 {
        ps.push(rng.range(1, WAD));
    }

    let mut worst_shortfall = BigRational::zero();
    let mut worst_p = 0i128;
    let mut clamped_low = 0usize;
    for p in ps {
        let p = p.clamp(1, WAD);
        let max = mirror_max_bonus(&env, p);
        let t = mul_div_ceil(&env, p, BPS, WAD).clamp(1, BPS);
        if mul_div_ceil(&env, p, BPS, WAD) < 1 {
            clamped_low += 1;
        }
        // Documented closed form at the BPS grid.
        assert_eq!(max, BPS * (BPS - t) / t, "closed form at p={p}");
        // `(1 + max) * t <= 1`, tight at the grid.
        assert!((BPS + max) * t <= BPS * BPS, "(1+max)*t > 1 at p={p}");
        assert!((BPS + max + 1) * t > BPS * BPS, "max not tight at p={p}");
        // HF-neutral at the half-up `p` itself: `(1 + max) * p <= 1`.
        assert!(
            (BPS + max) * p <= BPS * WAD,
            "max {max} breaks (1+max)*p <= 1 at raw p={p}"
        );
        // Never above the exact rational `BPS * (WAD - p) / p`.
        let exact = frac(big(BPS) * big(WAD - p), big(p));
        let got = rat(max);
        assert!(got <= exact, "max {max} above exact {exact} at p={p}");
        let short = &exact - &got;
        if short > worst_shortfall {
            worst_shortfall = short;
            worst_p = p;
        }
    }
    // The `clamp(1, BPS)` lower arm never fires for an admitted positive `p`:
    // `ceil(p * BPS / WAD) >= 1` for every `p >= 1`.
    assert_eq!(clamped_low, 0, "the lower clamp fired for a positive p");
    println!(
        "RV-R5 S1: threshold bound HF-neutral everywhere; largest shortfall below the exact \
         1/p - 1 is {:.3e} BPS at raw p={worst_p} (t ceil, conservative for the borrower)",
        f64_of(&worst_shortfall)
    );
}

// ---------------------------------------------------------------------------
// S2: HF-preserving cap vs C/D - 1
// ---------------------------------------------------------------------------

#[test]
fn rv_round_r5_hf_cap_overshoot_is_bounded_by_the_raw_rounding_slack() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    let mut rng = Rng(0x5eed_0002);

    let mut worst = BigRational::zero();
    let mut worst_case = None;
    let mut quantized_overshoots = 0usize;
    let mut cases = 0usize;
    for _ in 0..40_000 {
        let d = rng.log_range(10i128.pow(15), 10i128.pow(27));
        let c = if rng.coin() {
            // Within one BPS of par, where the band and the -1 cap live.
            rng.range(d, d + d / BPS + 1)
        } else {
            rng.range(d, 3 * d)
        };
        // Liquidatable: `floor(W / D) < 1` needs `W < D`; keep `p` realistic (>= 1%).
        let w = rng.range(c / 100, d - 1);
        let s = snap_of(&env, w, c, d);
        let Some(cap) = mirror_cap(&s) else {
            continue;
        };
        cases += 1;
        // Documented: "an account at C == D, or a few raw WAD units above it, can
        // compute a cap of -1"; never below that when solvent.
        assert!(cap >= -1, "solvent cap {cap} below -1 for {s:?}");
        if BPS + cap > c * BPS / d {
            quantized_overshoots += 1;
        }
        let over = assert_cap_within_slack(&s, cap, "random");
        if over > worst {
            worst = over;
            worst_case = Some((s, cap));
        }
    }
    assert!(cases > 30_000, "sweep too small: {cases}");

    // Targeted instance: `C * BPS / D` sits 1e-17 below the integer 10500, `HF` is
    // exact and `p` rounds down, so the BPS floor lands one above `floor(C*BPS/D)`.
    let d = 1_000 * WAD;
    let c = 1_050 * WAD - 1;
    let mut found = None;
    for i in 0..200i128 {
        let w = 800 * WAD + i * 1_000;
        let s = snap_of(&env, w, c, d);
        let cap = mirror_cap(&s).expect("hf < 1 and p > 0");
        if BPS + cap > c * BPS / d {
            found = Some((s, cap));
            break;
        }
    }
    let (s, cap) = found.expect("a quantized +1 BPS cap exists just below an integer C*BPS/D");
    assert_eq!(
        c * BPS / d,
        10_499,
        "floor(C*BPS/D) of the targeted instance"
    );
    assert_eq!(
        BPS + cap,
        10_500,
        "the executed 1 + cap is one BPS above that floor"
    );
    // ... but the seizure excess over `x * C / D` is the raw slack, not a BPS:
    // `(BPS + cap) * D - C * BPS = 10_000` raw out of `C * BPS = 1.05e25`.
    let excess_raw = (BPS + cap) * d - c * BPS;
    assert_eq!(
        excess_raw, 10_000,
        "excess of (1+cap)*D over C in raw BPS*WAD"
    );
    let over = assert_cap_within_slack(&s, cap, "targeted");
    assert!(
        over < frac(big(1), big(10i128.pow(20))),
        "targeted overshoot {over} is not below 1e-20"
    );
    println!(
        "RV-R5 S2: {cases} solvent snapshots, {quantized_overshoots} with 1+cap one BPS above \
         floor(C*BPS/D); worst relative excess of (1+cap) over C/D = {:.3e} at {:?}; targeted \
         W={} C={} D={}: HF={} p={} cap={} excess={} raw (relative {:.3e})",
        f64_of(&worst),
        worst_case,
        s.w,
        s.c,
        s.d,
        s.hf,
        s.p,
        cap,
        excess_raw,
        f64_of(&over)
    );
}

// ---------------------------------------------------------------------------
// S3: the band at par, cap -1 clamped to zero
// ---------------------------------------------------------------------------

#[test]
fn rv_round_r5_band_at_par_quotes_full_debt_at_zero_bonus_within_collateral() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    let mut rng = Rng(0x5eed_0003);
    let base = 500i128;

    let mut minus_one = 0usize;
    let mut zero = 0usize;
    let mut cases = 0usize;
    for _ in 0..20_000 {
        let d = rng.log_range(10i128.pow(15), 10i128.pow(27));
        let c = d + rng.range(0, 2_000);
        let w = rng.range(c / 2, d - 1);
        let s = snap_of(&env, w, c, d);
        let cap = mirror_cap(&s).expect("hf < 1 and p > 0");
        cases += 1;
        assert!(
            cap == -1 || cap == 0,
            "cap {cap} at C - D = {} raw for {s:?}",
            c - d
        );
        if cap == -1 {
            minus_one += 1;
        } else {
            zero += 1;
        }
        let max = mirror_max_bonus(&env, s.p);
        let (quote, bonus) = mirror_estimate(&env, &s, base.min(max), max, &DEFAULT_CURVE);
        // Band: the full debt at the cap clamped to zero.
        assert_eq!(bonus, 0, "band bonus for {s:?}");
        assert_eq!(quote, d, "band quote for {s:?}");
        // Seizure `D * (1 + 0) <= C`: the borrower keeps `C - D` (and the per-leg
        // token floors documented in F3/F6).
        assert!(quote <= c);
        assert_cap_within_slack(&s, cap, "par");
    }
    assert!(
        minus_one > 0 && zero > 0,
        "par sweep saw -1:{minus_one} 0:{zero}"
    );
    println!(
        "RV-R5 S3: {cases} par-band snapshots (C - D in [0, 2000] raw): cap -1 in {minus_one}, \
         cap 0 in {zero}; every quote (D, 0) with D <= C"
    );
}

// ---------------------------------------------------------------------------
// S4: USD-weighted half-up base bonus at the position limit
// ---------------------------------------------------------------------------

#[test]
fn rv_round_r5_base_bonus_within_half_bps_per_leg_and_never_above_threshold_bound() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    let mut rng = Rng(0x5eed_0004);
    let n = POSITION_LIMIT_MAX as usize;
    let half_per_leg = frac(big(n as i128), big(2));

    let mut worst_gain = BigRational::zero();
    let mut worst_loss = BigRational::zero();
    let mut capped = 0usize;
    for _ in 0..20_000 {
        let legs = rng.range(1, n as i128) as usize;
        let mut values = Vec::with_capacity(legs);
        let mut bonuses = Vec::with_capacity(legs);
        let mut lts = Vec::with_capacity(legs);
        for _ in 0..legs {
            values.push(rng.log_range(10i128.pow(12), 10i128.pow(26)));
            let lt = rng.range(1, BPS);
            // validate_risk_bounds: threshold * (BPS + bonus) <= BPS * BPS.
            let bonus_max = BPS * BPS / lt - BPS;
            let bonus = if rng.coin() {
                rng.range(0, bonus_max.min(5_000))
            } else {
                rng.range(0, bonus_max)
            };
            lts.push(lt);
            bonuses.push(bonus);
        }
        let c: i128 = values.iter().sum();
        // `apply_to_wad_floor` with `LT * 1e14` exact: `floor(value * LT / BPS)`.
        let w: i128 = values
            .iter()
            .zip(&lts)
            .map(|(v, lt)| mul_div_floor(&env, *v, *lt, BPS))
            .sum();
        let p = mul_div_half_up(&env, w, WAD, c);
        let max = mirror_max_bonus(&env, p);
        let got = mirror_base(&env, &values, &bonuses, c, max);
        assert!(got <= max, "base {got} above threshold bound {max}");
        assert!(got >= 0);
        let exact = exact_base(&values, &bonuses);
        let exact_capped = exact.clone().min(rat(max));
        if exact > rat(max) {
            capped += 1;
        }
        let diff = rat(got) - &exact_capped;
        assert!(
            diff <= half_per_leg && -diff.clone() <= half_per_leg,
            "base {got} is {diff} BPS from min(exact {exact}, max {max}) with {legs} legs"
        );
        if diff > worst_gain {
            worst_gain = diff.clone();
        }
        if -diff.clone() > worst_loss {
            worst_loss = -diff;
        }
    }

    // Adversarial composition at the limit: every per-leg product has fraction .5,
    // so each half-up adds exactly half a BPS: 5 legs gain 2.5 BPS over the exact
    // weighted average (250.5 -> 253). A borrower choosing fractions below .5 loses
    // the same 2.5 BPS the other way.
    let values = [21 * WAD, 21 * WAD, 21 * WAD, 27 * WAD, 10 * WAD];
    let bonuses = [250, 250, 250, 250, 255];
    let c: i128 = values.iter().sum();
    let w: i128 = values
        .iter()
        .map(|v| mul_div_floor(&env, *v, 8_000, BPS))
        .sum();
    let p = mul_div_half_up(&env, w, WAD, c);
    let max = mirror_max_bonus(&env, p);
    assert_eq!(max, 2_500, "LT 8000 everywhere gives max 2500");
    let got = mirror_base(&env, &values, &bonuses, c, max);
    let exact = exact_base(&values, &bonuses);
    assert_eq!(
        exact * rat(2),
        rat(501),
        "exact weighted average is 250.5 BPS"
    );
    assert_eq!(got, 253, "five half-ups of x.5 gain 2.5 BPS");

    println!(
        "RV-R5 S4: random legs <= {n}: base within +{:.3} / -{:.3} BPS of the exact weighted \
         average (bound {n}/2 = {}), {capped} cases capped at the threshold bound; adversarial \
         5-leg book: exact 250.5 -> executed 253 (+2.5 BPS = N/2)",
        f64_of(&worst_gain),
        f64_of(&worst_loss),
        f64_of(&half_per_leg)
    );
}

// ---------------------------------------------------------------------------
// S5: curve bonus within one BPS of the exact ramp
// ---------------------------------------------------------------------------

#[test]
fn rv_round_r5_curve_bonus_within_one_bps_of_exact_ramp_and_inside_bounds() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    let mut rng = Rng(0x5eed_0005);
    let one_bps = rat(1) + frac(big(1), big(1_000_000));

    let mut worst = BigRational::zero();
    let mut cases = 0usize;
    for _ in 0..40_000 {
        // validate_liquidation_curve: H in (WAD, 10 WAD], K in (0, H), f in (0, BPS].
        let cv = if rng.coin() {
            DEFAULT_CURVE
        } else {
            let h = rng.range(WAD + 1, MAX_LIQUIDATION_TARGET_HF_WAD);
            Curve {
                h,
                k: rng.range(1, h - 1),
                f: rng.range(1, BPS),
            }
        };
        let hf = rng.range(0, WAD - 1);
        let p = rng.range(1, WAD);
        let max = mirror_max_bonus(&env, p);
        let base = if rng.coin() {
            rng.range(0, max.min(3_000))
        } else {
            rng.range(0, max)
        };
        let got = mirror_curve(&env, hf, base, max, &cv);
        assert!(
            got >= base && got <= max,
            "curve {got} outside [{base}, {max}]"
        );
        // Exact: base + f * (max - base) * min(1, (H - HF) / (H - K)) / BPS.
        let s = frac(big(cv.h - hf), big(cv.h - cv.k)).min(rat(1));
        let exact = rat(base) + rat(cv.f) * rat(max - base) * s / rat(BPS);
        let diff = rat(got) - exact;
        let abs = if diff < BigRational::zero() {
            -diff
        } else {
            diff
        };
        assert!(
            abs <= one_bps,
            "curve {got} is {abs} BPS from exact at hf={hf} {cv:?}"
        );
        if abs > worst {
            worst = abs;
        }
        cases += 1;
    }
    println!(
        "RV-R5 S5: {cases} curve points: |executed - exact ramp| <= {:.4} BPS (two half-ups, \
         bound 1), always inside [base, max]",
        f64_of(&worst)
    );
}

// ---------------------------------------------------------------------------
// S6: the whole estimate: bonus caps and seizure within collateral
// ---------------------------------------------------------------------------

#[test]
fn rv_round_r5_estimate_never_exceeds_caps_or_collateral_beyond_raw_slack() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    let mut rng = Rng(0x5eed_0006);

    let mut branches = [0usize; 5]; // general, promoted, band, insolvent, none
    let mut worst_seizure_excess = BigRational::zero();
    for _ in 0..40_000 {
        let d = rng.log_range(10i128.pow(15), 10i128.pow(27));
        let c = match rng.next() % 4 {
            0 => rng.range(d / 2, d - 1),
            1 => rng.range(d, d + d / BPS + 1),
            2 => rng.range(d, d + d / 10),
            _ => rng.range(d, 3 * d),
        };
        let w_hi = c.min(d) - 1;
        // Admitted region: every leg has LT >= 1 BPS, floor valuation at most one
        // raw unit below half-up, at most POSITION_LIMIT_MAX legs, so
        // `W >= (C - 5) / BPS - 5`. Below that `p` is raw dust and the cap's
        // integer rounding is unbounded (an implied LT below 1e-14).
        let w_lo = ((c - 5) / BPS - 5).max(0).min(w_hi);
        let w = if rng.next().is_multiple_of(8) {
            rng.range(w_lo, (w_lo + 10i128.pow(6)).min(w_hi))
        } else {
            rng.range(w_lo, w_hi)
        };
        let s = snap_of(&env, w, c, d);
        let max = mirror_max_bonus(&env, s.p);
        let base = if rng.coin() {
            rng.range(0, max.min(3_000))
        } else {
            rng.range(0, max)
        };
        let cv = if rng.coin() {
            DEFAULT_CURVE
        } else {
            let h = rng.range(WAD + 1, MAX_LIQUIDATION_TARGET_HF_WAD);
            Curve {
                h,
                k: rng.range(1, h - 1),
                f: rng.range(1, BPS),
            }
        };
        let (quote, bonus) = mirror_estimate(&env, &s, base, max, &cv);

        // Executed bonus never above the threshold bound, never negative.
        assert!(
            bonus >= 0 && bonus <= max,
            "bonus {bonus} outside [0, {max}] for {s:?}"
        );
        assert!(
            quote >= 0 && quote <= d,
            "quote {quote} outside [0, D] for {s:?}"
        );

        let cap = mirror_cap(&s);
        match cap {
            None => {
                branches[4] += 1;
                assert_eq!(s.p, 0);
                assert_eq!(bonus, 0, "p == 0 gives max 0 and a zero bonus");
            }
            Some(_) if c < d => {
                branches[3] += 1;
                assert_eq!(bonus, base, "insolvent quote is at the base bonus");
                // floor(C / (1 + base)) * (1 + base) <= C exactly.
                let lhs = rat(quote) * rat(WAD + bps_to_wad(&env, base));
                assert!(
                    lhs <= rat(c) * rat(WAD),
                    "insolvent seizure above C for {s:?}"
                );
            }
            Some(cap) if cap < base => {
                branches[2] += 1;
                assert_eq!(bonus, cap.max(0));
                assert_eq!(quote, d);
                // `HF >= floor(A / C) >= p - 1`, so `cap >= -ceil(BPS / p)`: -1 for
                // every `p >= 1e-14`, down to `-BPS` only when `W` is raw dust.
                assert!(cap >= -((BPS + s.p - 1) / s.p), "band cap {cap} for {s:?}");
            }
            Some(cap) => {
                if quote == d {
                    branches[1] += 1;
                } else {
                    branches[0] += 1;
                }
                let curve = mirror_curve(&env, s.hf, base, max, &cv);
                assert_eq!(
                    bonus,
                    curve.min(cap),
                    "general-branch bonus is min(curve, cap)"
                );
            }
        }

        // Seizure within collateral: `quote * (1 + b) <= C * slack + (1 + b) / 2`
        // where slack is the cap's `2A / (2A - C)` and the half raw unit is
        // `d_max`'s half-up (curve.rs line 173).
        let one_plus = rat(WAD + bps_to_wad(&env, bonus));
        let seized = rat(quote) * &one_plus;
        let a2 = big(s.w) * big(WAD) * big(2);
        let slack = if a2 > big(s.c) {
            frac(a2.clone(), &a2 - big(s.c))
        } else {
            rat(1)
        };
        let bound = rat(c) * rat(WAD) * &slack + &one_plus / rat(2) + rat(1);
        assert!(
            seized <= bound,
            "seizure {seized} above {bound} for {s:?} q={quote} b={bonus}"
        );
        if c > 0 {
            let excess = &seized / (rat(c) * rat(WAD)) - rat(1);
            // Documented: at most the raw rounding slack, about `1 / (2 p_raw)`
            // with `p_raw >= 1e14` in the admitted region.
            assert!(
                excess <= frac(big(1), big(10i128.pow(13))),
                "seizure exceeds C by {excess} (relative) for {s:?} q={quote} b={bonus}"
            );
            if excess > worst_seizure_excess {
                worst_seizure_excess = excess;
            }
        }

        // Solvent partial: C/D never falls beyond the cap slack. With
        // `q * (1 + b) * D <= q * C * 2A / (2A - C)`,
        // `(C - q(1+b)) * D >= C * (D - q) - q * C * C / (2A - C)`.
        if c >= d && quote < d && quote > 0 && a2 > big(s.c) {
            let lhs = (rat(c) * rat(WAD) - &seized) * rat(d);
            let rhs = rat(c) * rat(WAD) * rat(d - quote)
                - rat(quote) * rat(c) * rat(c) * rat(WAD) / frac(&a2 - big(s.c), big(1));
            assert!(
                lhs >= rhs,
                "solvent partial lowered C/D beyond slack for {s:?}"
            );
        }
    }
    println!(
        "RV-R5 S6: branches general={} promoted-to-full={} band={} insolvent={} p0={}; \
         worst seizure excess over C = {:.3e} relative (bound: cap slack + half a raw WAD)",
        branches[0],
        branches[1],
        branches[2],
        branches[3],
        branches[4],
        f64_of(&worst_seizure_excess)
    );
}

// ---------------------------------------------------------------------------
// S7: the mirror against the live controller
// ---------------------------------------------------------------------------

const DEBT: &str = "USDT";

struct Leg {
    name: &'static str,
    price: i128,
    supply_raw: i128,
    lt: u32,
    bonus: u32,
}

const LEGS: [Leg; 3] = [
    Leg {
        name: "CA",
        price: WAD,
        supply_raw: 10_000 * 10_000_000,
        lt: 8_500,
        bonus: 400,
    },
    Leg {
        name: "CB",
        price: 2_000 * WAD,
        supply_raw: 2 * 10_000_000,
        lt: 7_800,
        bonus: 800,
    },
    Leg {
        name: "CC",
        price: 60_000 * WAD,
        supply_raw: 1_000_000,
        lt: 7_500,
        bonus: 1_000,
    },
];

fn build(target_hf: f64) -> LendingTest {
    let mut b = LendingTest::new();
    for l in LEGS.iter() {
        b = b.with_market(MarketPreset {
            name: l.name,
            decimals: 7,
            price_wad: l.price,
            initial_liquidity: 1_000.0,
            config: AssetConfigPreset {
                loan_to_value: l.lt - 500,
                liquidation_threshold: l.lt,
                liquidation_bonus: l.bonus,
                liquidation_fees: 1_000,
                ..DEFAULT_ASSET_CONFIG
            },
            params: DEFAULT_MARKET_PARAMS,
        });
    }
    b = b.with_market(MarketPreset {
        name: DEBT,
        decimals: 7,
        price_wad: WAD,
        initial_liquidity: 10_000_000.0,
        config: AssetConfigPreset {
            loan_to_value: 9_000,
            liquidation_threshold: 9_500,
            liquidation_bonus: 200,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    });
    let mut t = b.build();
    for l in LEGS.iter() {
        t.supply_raw(ALICE, l.name, l.supply_raw);
    }
    t.borrow(ALICE, DEBT, 12_000.0);
    let acc = t.resolve_account_id(ALICE);
    let hf0 = t.ctrl_client().get_health_factor(&acc) as f64 / WAD as f64;
    let factor = target_hf / hf0;
    for l in LEGS.iter() {
        t.set_price(l.name, (l.price as f64 * factor).round() as i128);
    }
    t.get_or_create_user(LIQUIDATOR);
    t.assert_liquidatable(ALICE);
    t
}

fn key(t: &LendingTest, name: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(name))
}

/// `PriceFeedRaw::price_wad` as `Context::cached_price` loads it from the aggregator.
fn aggregator_price(t: &LendingTest, name: &str) -> i128 {
    let asset = t.resolve_asset(name);
    let mut keys = soroban_sdk::Vec::new(&t.env);
    keys.push_back(PriceKey::Token(asset.clone()));
    t.price_agg_client()
        .prices(&keys)
        .get(PriceKey::Token(asset))
        .expect("aggregator feed")
        .price_wad
}

fn raw_position(t: &LendingTest, id: u64, k: &HubAssetKey) -> AccountPositionRaw {
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .get::<_, Map<HubAssetKey, AccountPositionRaw>>(&ControllerKey::SupplyPositions(id))
            .and_then(|book| book.get(k.clone()))
            .expect("supply position")
    })
}

/// Runs one regime: rebuilds the mirror from storage and the views, compares
/// the bonus exactly and the quote within one debt-token unit (the trim).
fn check_regime(target_hf: f64, label: &str) {
    let t = build(target_hf);
    let env = &t.env;
    let acc = t.resolve_account_id(ALICE);
    let c = t.ctrl_client().get_total_collateral_usd(&acc);
    let d = t.ctrl_client().get_total_borrow_usd(&acc);
    let w = t.ctrl_client().get_liquidation_collateral(&acc);
    let hf = t.ctrl_client().get_health_factor(&acc);

    let mut values = Vec::new();
    let mut bonuses = Vec::new();
    for l in LEGS.iter() {
        let k = key(&t, l.name);
        let raw = raw_position(&t, acc, &k);
        // The risk totals value positions at the index projected to this ledger
        // (`cached_market_index`), which `get_market_index` exposes; the pool's
        // stored index is stale by the accrual since the last sync.
        let index = t.ctrl_client().get_market_index(&k).supply_index;
        // The price the controller caches is the aggregator feed (14-decimal
        // Reflector granularity), not the harness's recorded WAD value.
        let price = aggregator_price(&t, l.name);
        // `position_value`, rates/value.rs lines 15-19: half-up at each step.
        let value = Ray::from(raw.scaled_amount)
            .mul(env, Ray::from(index))
            .to_wad(env)
            .mul(env, Wad::from(price));
        values.push(value.raw());
        bonuses.push(i128::from(raw.liquidation_bonus));
    }
    let c_sum: i128 = values.iter().sum();
    assert_eq!(
        c_sum, c,
        "{label}: per-leg half-up values sum to the view's C"
    );

    let s = snap_of(env, w, c, d);
    assert_eq!(s.hf, hf, "{label}: mirrored HF equals the view");
    let max = mirror_max_bonus(env, s.p);
    let base = mirror_base(env, &values, &bonuses, c, max);
    let (quote, bonus) = mirror_estimate(env, &s, base, max, &DEFAULT_CURVE);

    let debt_raw = t.ctrl_client().get_borrow_amount(&acc, &key(&t, DEBT));
    let mut pays = soroban_sdk::Vec::new(env);
    pays.push_back((key(&t, DEBT), 2 * debt_raw));
    let e = t
        .ctrl_client()
        .get_liquidation_estimate(&acc, &pays, &SeizeMode::Transfer);

    let unit_usd = WAD / 10_000_000; // one USDT base unit, $1e-7
    println!(
        "RV-R5 S7 {label}: C={c} D={d} W={w} HF={hf} p={} max={max} base={base} cap={:?} -> \
         mirror (quote={quote}, bonus={bonus}) vs controller (max_payment={}, bonus={})",
        s.p,
        mirror_cap(&s),
        e.max_payment_wad,
        e.bonus_rate_bps
    );
    assert_eq!(
        e.bonus_rate_bps, bonus,
        "{label}: executed bonus equals the mirror"
    );
    assert!(
        (e.max_payment_wad - quote).abs() < unit_usd,
        "{label}: controller quote {} differs from mirror {quote} by a debt unit or more",
        e.max_payment_wad
    );
}

#[test]
fn rv_round_r5_live_controller_matches_the_mirror_in_every_branch() {
    // General branch (solvent, cap above base): HF 0.95 with p ~ 0.8 gives C/D ~ 1.19.
    check_regime(0.95, "general");
    // Band (cap below base): HF 0.82 gives C/D ~ 1.02, cap ~ 250 < base ~ 616.
    check_regime(0.82, "band");
    // Insolvent: HF 0.76 gives C/D ~ 0.95.
    check_regime(0.76, "insolvent");
}
