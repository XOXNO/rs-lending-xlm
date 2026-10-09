//! RV rounding audit, lens R-9, round 1: valuation and health rounding.
//!
//! Integer mirrors of `common/src/rates/value.rs` (`position_value`,
//! `position_value_floor`, `position_value_ceil`, lines 15-39),
//! `contracts/pool/src/guards.rs` (`require_utilization_below_max` lines 19-34,
//! `require_liquidation_buffer` lines 39-47) and `common/src/oracle/lp.rs`
//! (`fair_lp_price_wad` lines 57-97), checked against exact rationals, plus the
//! live controller for the health gates in `contracts/controller/src/risk`
//! (`totals.rs` lines 157-216, `validation.rs` lines 29-61).
//!
//! Every assertion is the documented bound (`docs/reference/formulas.md`,
//! "Valuation and health", "Backing and cash constraints"):
//! * per leg the three-step chain floors below, ceils above and half-ups within
//!   half a unit per step of the exact value; the ceil-floor gap is at most
//!   `ceil(P / WAD) + 2` raw WAD, i.e. `1e-9` USD at the `$1e9` price cap;
//! * the liquidation buffer is `ceil(2% * floor(supply))`, within one token unit
//!   below the exact ceiling and never below it by more, and the utilization gate
//!   ratio is never below the exact ratio;
//! * an LP share price is within the half-up reserve values' propagated slack of
//!   `2 * sqrt(a * b) * WAD / supply`;
//! * an account that passes the post-pool gate has HF >= 1 in the same ledger,
//!   HF is exactly one WAD at exact parity (not liquidatable) and one price tick
//!   below is liquidatable; the HF view equals `floor(W * WAD / ceil(D))` bit for
//!   bit after accrual;
//! * the `$5` minimum LTV-collateral floor binds at the exact base unit;
//! * repaying the half-up displayed debt can leave sub-unit debt, the ceiled
//!   valuation keeps the account in debt and the `$5` floor then gates
//!   withdrawals until one more base unit is repaid (documented: display half-up
//!   versus ceil on close);
//! * at `C == D` the band quotes `(D, 0)`; one tick below the insolvent branch
//!   quotes `floor(C / (1 + base))` at `base`.

use common::constants::{BPS, LIQUIDATION_BUFFER_BPS, MAX_REASONABLE_PRICE_WAD, RAY, WAD};
use common::math::fp::{Ray, Wad};
use common::math::fp_core::{mul_div_ceil, mul_div_floor};
use common::oracle::lp::{fair_lp_price_wad, LpLeg, LpSupply};
use common::rates::{
    position_value, position_value_ceil, position_value_floor, unscale_supply_floor,
};
use common::types::{HubAssetKey, SeizeMode};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};
use soroban_sdk::{vec, Env};
use test_harness::{
    assert_contract_error, errors, hub_asset, usdc_preset, usdt_stable_preset, AssetConfigPreset,
    LendingTest, MarketPreset, ALICE, BOB, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS,
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

    /// Uniform in `[lo, hi]` for `0 <= lo <= hi`; `hi` may be `i128::MAX`.
    fn range(&mut self, lo: i128, hi: i128) -> i128 {
        assert!(lo >= 0 && hi >= lo, "range: lo={lo} hi={hi}");
        let span = (hi - lo) as u128 + 1;
        let r = (((self.next() as u128) << 64) | self.next() as u128) % span;
        lo + r as i128
    }

    /// Log-uniform over the decades of `[lo, hi]` (`lo > 0`).
    fn log_range(&mut self, lo: i128, hi: i128) -> i128 {
        let decades = (lo.ilog10() as i128, hi.ilog10() as i128);
        let e = self.range(decades.0, decades.1) as u32;
        let base = 10i128.pow(e);
        let top = base.saturating_mul(10).saturating_sub(1).min(hi);
        self.range(base, top).clamp(lo, hi)
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

fn pow10(e: u32) -> BigInt {
    BigInt::from(10u32).pow(e)
}

/// `exact * WAD / denominator` style helper: `a * b / c` as a rational.
fn mul_div_rat(a: i128, b: i128, c: &BigInt) -> BigRational {
    frac(big(a) * big(b), c.clone())
}

/// `floor(sqrt(n))` for `n >= 0` by Newton's method from above.
fn isqrt_big(n: &BigInt) -> BigInt {
    if n <= &big(1) {
        return n.clone();
    }
    let two = big(2);
    let mut x = BigInt::from(1u8) << n.bits().div_ceil(2);
    loop {
        let y = (&x + n / &x) / &two;
        if y >= x {
            return x;
        }
        x = y;
    }
}

fn f64_of(r: &BigRational) -> f64 {
    r.numer().to_f64().unwrap_or(f64::NAN) / r.denom().to_f64().unwrap_or(f64::NAN)
}

// ---------------------------------------------------------------------------
// A. The three-step valuation chain, per leg, against exact rationals
// ---------------------------------------------------------------------------

/// Decimals the brief names: 0 and 18 extremes plus the listed middle values.
const DECIMALS: [u32; 6] = [0, 2, 3, 6, 7, 18];

/// Prices in WAD: one raw unit (`$1e-18`), `$1e-15`, `$1e-6`, `$1`, `$1e3`,
/// and the sanity cap `$1e9` (`MAX_REASONABLE_PRICE_WAD`).
const PRICES: [i128; 6] = [
    1,
    1_000,
    1_000_000_000_000,
    WAD,
    1_000 * WAD,
    MAX_REASONABLE_PRICE_WAD,
];

/// Indexes: the write-down floor, one, a typical accrued value and the ceiling.
const INDEXES: [i128; 4] = [
    RAY / 1_000,
    RAY,
    RAY + RAY / 2,
    1_000_000_000_000_000_000_000_000_000_000_000_000,
];

/// Largest scaled amount whose ceiled value stays inside `i128` for `(index,
/// price)`: the RAY asset value must fit and so must the WAD USD value.
fn scaled_upper_bound(index: i128, price: i128) -> i128 {
    // Asset value in RAY: scaled * index / RAY <= i128::MAX / 2 (headroom).
    let by_ray = (big(i128::MAX / 2) * big(RAY)) / big(index);
    // USD value in WAD: asset_wad * price / WAD <= i128::MAX / 2, where
    // asset_wad = scaled * index / RAY / 1e9.
    let by_usd = (big(i128::MAX / 2) * big(WAD) * big(RAY) * pow10(9)) / (big(index) * big(price));
    let bound = by_ray.min(by_usd);
    bound.to_i128().unwrap_or(i128::MAX).max(1)
}

#[test]
fn rv_round_9_valuation_chain_floor_half_up_ceil_bracket_exact_within_one_step_each() {
    let env = Env::default();
    let mut rng = Rng(0x9A5E_D911_C0DE_0001);
    let ray_wad_scale = big(RAY) * pow10(9) * big(WAD); // s * i * p / this = exact WAD USD

    let mut cases = 0u64;
    let mut max_gap_raw = big(0);
    let mut max_gap_at = (0i128, 0i128, 0i128);
    let mut max_rel_gap = BigRational::zero();
    // Relative ceil-floor gap on positions worth at least one dollar.
    let mut max_rel_gap_dollar = BigRational::zero();

    for &decimals in DECIMALS.iter() {
        let unit_ray = pow10(27 - decimals); // one base unit in RAY asset units
        for &price in PRICES.iter() {
            for &index in INDEXES.iter() {
                let upper = scaled_upper_bound(index, price);
                for sample in 0..40 {
                    // Half the samples are whole base-unit multiples at this
                    // index (what a deposit mints), half are arbitrary shares.
                    let scaled = if sample % 2 == 0 {
                        let units_cap = (big(upper) * big(index) / (big(RAY) * unit_ray.clone()))
                            .to_i128()
                            .unwrap_or(i128::MAX);
                        if units_cap < 1 {
                            continue;
                        }
                        let units = rng.log_range(1, units_cap);
                        // shares = floor(units_ray / index), as calculate_scaled_supply
                        let shares = (big(units) * unit_ray.clone() * big(RAY)) / big(index);
                        let Some(shares) = shares.to_i128() else {
                            continue;
                        };
                        if shares < 1 {
                            continue;
                        }
                        shares.min(upper)
                    } else {
                        rng.log_range(1, upper)
                    };

                    let s = Ray::from(scaled);
                    let i = Ray::from(index);
                    let p = Wad::from(price);
                    let floor = position_value_floor(&env, s, i, p).raw();
                    let half = position_value(&env, s, i, p).raw();
                    let ceil = position_value_ceil(&env, s, i, p).raw();

                    let exact = frac(big(scaled) * big(index) * big(price), ray_wad_scale.clone());
                    let p_over_wad = frac(big(price), big(WAD));

                    // Bracketing: floor <= exact <= ceil, floor <= half-up <= ceil.
                    assert!(
                        rat(floor) <= exact && exact <= rat(ceil),
                        "d={decimals} p={price} i={index} s={scaled}: floor {floor} <= exact {} <= ceil {ceil}",
                        f64_of(&exact)
                    );
                    assert!(
                        floor <= half && half <= ceil,
                        "d={decimals} p={price} i={index} s={scaled}: floor {floor} <= half {half} <= ceil {ceil}"
                    );

                    // Per-step bounds. Step 1 loses < 1 RAY unit, step 2 < 1 WAD
                    // asset unit (plus the propagated 1e-9), step 3 < 1 raw WAD USD.
                    let one_sided = rat(1) + &p_over_wad * (rat(1) + frac(big(1), pow10(9)));
                    assert!(
                        &exact - rat(floor) < one_sided,
                        "d={decimals} p={price} i={index} s={scaled}: floor short by {} > {}",
                        f64_of(&(&exact - rat(floor))),
                        f64_of(&one_sided)
                    );
                    assert!(
                        rat(ceil) - &exact < one_sided,
                        "d={decimals} p={price} i={index} s={scaled}: ceil over by {} > {}",
                        f64_of(&(rat(ceil) - &exact)),
                        f64_of(&one_sided)
                    );
                    let half_bound = frac(big(1), big(2))
                        + &p_over_wad * (frac(big(1), big(2)) + frac(big(1), pow10(9)));
                    assert!(
                        (rat(half) - &exact).abs() <= half_bound,
                        "d={decimals} p={price} i={index} s={scaled}: half-up off by {} > {}",
                        f64_of(&(rat(half) - &exact).abs()),
                        f64_of(&half_bound)
                    );

                    // Gap bound: ceil - floor <= ceil(P / WAD) + 2 raw WAD.
                    let gap = big(ceil - floor);
                    let gap_bound = big((price + WAD - 1) / WAD + 2);
                    assert!(
                        gap <= gap_bound,
                        "d={decimals} p={price} i={index} s={scaled}: gap {gap} > {gap_bound}"
                    );
                    if gap > max_gap_raw {
                        max_gap_raw = gap.clone();
                        max_gap_at = (decimals as i128, price, index);
                    }
                    if exact > BigRational::zero() {
                        let rel = frac(gap.clone(), big(1)) / &exact;
                        if rel > max_rel_gap {
                            max_rel_gap = rel.clone();
                        }
                        if exact >= rat(WAD) && rel > max_rel_gap_dollar {
                            max_rel_gap_dollar = rel;
                        }
                    }
                    cases += 1;
                }
            }
        }
    }
    println!(
        "RV-R9 A: {cases} cases; max ceil-floor gap {max_gap_raw} raw WAD (1e-18 USD) at \
         (decimals, price, index) = {max_gap_at:?}; max relative gap {:.3e} (sub-dollar dust), \
         {:.3e} on positions worth >= $1",
        f64_of(&max_rel_gap),
        f64_of(&max_rel_gap_dollar)
    );
    assert!(cases > 4_000, "sweep too small: {cases}");
    // The largest absolute gap is one base unit of value at the price cap: $1e-9.
    assert!(
        max_gap_raw <= big(MAX_REASONABLE_PRICE_WAD / WAD + 2),
        "max gap {max_gap_raw} exceeds the price-cap bound"
    );
    // On a dollar or more the ceil-floor window is at most (P/WAD + 2) / WAD,
    // i.e. about 1e-9 relative at the $1e9 price cap and 1e-18 at $1.
    assert!(
        max_rel_gap_dollar <= frac(big(MAX_REASONABLE_PRICE_WAD / WAD + 2), big(WAD)),
        "relative gap on >= $1 positions {:.3e} exceeds (P/WAD + 2)/WAD",
        f64_of(&max_rel_gap_dollar)
    );
}

// ---------------------------------------------------------------------------
// B. The pool's liquidation buffer and utilization gate, against exact ratios
// ---------------------------------------------------------------------------

#[test]
fn rv_round_9_liquidation_buffer_within_one_unit_below_exact_and_utilization_gate_never_below_exact(
) {
    let env = Env::default();
    let mut rng = Rng(0x9A5E_D911_C0DE_0002);
    let mut cases = 0u64;
    let mut max_over_reserve_bps = 0i128;
    let mut max_over_reserve_at = (0i128, 0i128);

    for &decimals in DECIMALS.iter() {
        let unit_ray = pow10(27 - decimals);
        for &si in INDEXES.iter() {
            for sample in 0..60 {
                // Supply shares: small pools (1..1000 base units) half the time.
                let supplied = if sample % 2 == 0 {
                    let units = rng.range(1, 1_000);
                    let shares = (big(units) * unit_ray.clone() * big(RAY)) / big(si);
                    let Some(shares) = shares.to_i128() else {
                        continue;
                    };
                    if shares < 1 {
                        continue;
                    }
                    shares
                } else {
                    rng.log_range(1, scaled_upper_bound(si, WAD))
                };

                // Buffer: formulas.md "rounded up from the floored supplied token value".
                let floor_units =
                    unscale_supply_floor(&env, Ray::from(supplied), Ray::from(si), decimals);
                let reserved = mul_div_ceil(&env, floor_units, LIQUIDATION_BUFFER_BPS, BPS);
                // Exact supplied token units and exact ceil(2%).
                let exact_units = frac(big(supplied) * big(si), big(RAY) * unit_ray.clone());
                let exact_reserved = (exact_units * rat(LIQUIDATION_BUFFER_BPS) / rat(BPS)).ceil();
                let exact_reserved = exact_reserved.to_integer();
                assert!(
                    big(reserved) <= exact_reserved,
                    "d={decimals} si={si} s={supplied}: reserved {reserved} above exact {exact_reserved}"
                );
                assert!(
                    big(reserved) + big(1) >= exact_reserved,
                    "d={decimals} si={si} s={supplied}: reserved {reserved} more than one unit below exact {exact_reserved}"
                );
                if floor_units >= 1 {
                    assert!(
                        reserved >= 1,
                        "a positive floored supply reserves at least one unit"
                    );
                    let over_bps = reserved * BPS / floor_units;
                    if over_bps > max_over_reserve_bps {
                        max_over_reserve_bps = over_bps;
                        max_over_reserve_at = (floor_units, reserved);
                    }
                }

                // Utilization gate: ceil(ceil(debt) / floor(supply)) >= exact.
                // Backing keeps debt value at or below supply value; sample up to
                // 101% utilization so the gate's boundary is covered.
                let bi = INDEXES[rng.range(0, 3) as usize];
                let max_borrowed = (big(supplied) * big(si) * big(101) / (big(bi) * big(100)))
                    .to_i128()
                    .unwrap_or(i128::MAX)
                    .min(scaled_upper_bound(bi, WAD));
                let borrowed = rng.range(0, max_borrowed.max(0));
                if borrowed == 0 {
                    cases += 1;
                    continue;
                }
                let debt_ray = Ray::from(borrowed).mul_ceil(&env, Ray::from(bi));
                let supply_ray = Ray::from(supplied).mul_floor(&env, Ray::from(si));
                if supply_ray <= Ray::ZERO {
                    // formulas.md: "debt against a zero floored supply value fails it".
                    cases += 1;
                    continue;
                }
                let gate = debt_ray.div_ceil(&env, supply_ray).raw();
                let exact_ratio = frac(big(borrowed) * big(bi), big(supplied) * big(si));
                assert!(
                    rat(gate) >= &exact_ratio * rat(RAY),
                    "si={si} bi={bi} s={supplied} b={borrowed}: gate {gate} below exact utilization"
                );
                // Overshoot: (x+1)/(y-1) - x/y = (x+y)/(y(y-1)) in RAY asset units, plus the final ceil.
                let x = mul_div_rat(borrowed, bi, &big(RAY));
                let y = mul_div_rat(supplied, si, &big(RAY));
                if y > rat(1) {
                    let overshoot = (&x + &y) / (&y * (&y - rat(1)));
                    let bound = &exact_ratio * rat(RAY) + overshoot * rat(RAY) + rat(1);
                    assert!(
                        rat(gate) <= bound,
                        "si={si} bi={bi} s={supplied} b={borrowed}: gate {gate} above the documented overshoot"
                    );
                }
                cases += 1;
            }
        }
    }
    println!(
        "RV-R9 B: {cases} cases; buffer within [exact-1, exact]; largest over-reservation on a \
         tiny pool {max_over_reserve_bps} BPS of floored supply at (floored units, reserved) = \
         {max_over_reserve_at:?} (ceil of a sub-unit 2% reserves a whole unit)"
    );
    assert!(cases > 1_000, "sweep too small: {cases}");
}

// ---------------------------------------------------------------------------
// C. LP fair value against the exact 2*sqrt(a*b) formula
// ---------------------------------------------------------------------------

#[test]
fn rv_round_9_lp_fair_value_within_propagated_half_up_slack_of_exact() {
    let env = Env::default();
    let mut rng = Rng(0x9A5E_D911_C0DE_0003);
    let mut cases = 0u64;
    let mut max_err_raw = BigRational::zero();
    let mut max_rel_err_funded = BigRational::zero();

    for &da in [0u32, 6, 7, 18].iter() {
        for &db in [0u32, 7, 18].iter() {
            for &dsup in [7u32, 18].iter() {
                for _ in 0..25 {
                    let reserve_a = rng.log_range(1, 10i128.pow(da + 9));
                    let reserve_b = rng.log_range(1, 10i128.pow(db + 9));
                    let price_a = rng.log_range(1_000, 1_000 * WAD);
                    let price_b = rng.log_range(1_000, 1_000 * WAD);
                    let total_shares = rng.log_range(1, 10i128.pow(dsup + 9));
                    let a = LpLeg {
                        reserve: reserve_a,
                        decimals: da,
                        price_wad: price_a,
                    };
                    let b = LpLeg {
                        reserve: reserve_b,
                        decimals: db,
                        price_wad: price_b,
                    };
                    let supply = LpSupply {
                        total_shares,
                        decimals: dsup,
                    };
                    let Ok(fair) = fair_lp_price_wad(&env, &a, &b, &supply) else {
                        continue;
                    };

                    // Exact leg values in WAD USD and share supply in WAD.
                    let va = frac(big(reserve_a) * big(price_a), pow10(da));
                    let vb = frac(big(reserve_b) * big(price_b), pow10(db));
                    let s_wad = big(total_shares) * pow10(18 - dsup);
                    // sqrt(va*vb) bracketed by integer roots of floor(va*vb).
                    let prod = (&va * &vb).floor().to_integer();
                    let lo = isqrt_big(&prod);
                    let hi = &lo + big(2); // sqrt(n+1) < isqrt(n) + 2
                    let t_lo = rat(2) * BigRational::from_integer(lo.clone());
                    let t_hi = rat(2) * BigRational::from_integer(hi);
                    let scale = frac(big(WAD), s_wad.clone());
                    // Half-up reserve values move sqrt(ab) by at most
                    // ((a+b)/2 + 1/4) / sqrt(ab); isqrt floors by < 1; final floor by < 1.
                    // sqrt(ab) >= isqrt(floor(ab)) when ab >= 1, and >= ab when ab < 1.
                    let sqrt_lb = if lo.is_zero() {
                        &va * &vb
                    } else {
                        BigRational::from_integer(lo.clone())
                    };
                    let slack =
                        (rat(2) + ((&va + &vb) + frac(big(1), big(2))) / sqrt_lb) * &scale + rat(1);
                    let lower = &t_lo * &scale - &slack;
                    let upper = &t_hi * &scale + &slack;
                    assert!(
                        rat(fair) >= lower && rat(fair) <= upper,
                        "a=({reserve_a},{da},{price_a}) b=({reserve_b},{db},{price_b}) s=({total_shares},{dsup}): fair {fair} outside [{}, {}]",
                        f64_of(&lower),
                        f64_of(&upper)
                    );
                    let mid = (&t_lo + &t_hi) / rat(2) * &scale;
                    let err = (rat(fair) - &mid).abs();
                    if err > max_err_raw {
                        max_err_raw = err.clone();
                    }
                    // Relative error on a funded pool: both legs worth >= $1 and
                    // a share worth >= $1e-9. Half-up reserve values contribute
                    // <= 0.5/va + 0.5/vb, isqrt < 1/sqrt(ab), the final floor
                    // < 1/fair: together under 2e-9 relative.
                    if va >= rat(WAD) && vb >= rat(WAD) && fair >= 1_000_000_000 && mid > rat(0) {
                        let rel = &err / &mid;
                        if rel > max_rel_err_funded {
                            max_rel_err_funded = rel;
                        }
                    }
                    cases += 1;
                }
            }
        }
    }
    println!(
        "RV-R9 C: {cases} cases; max |fair - 2sqrt(ab)*WAD/S| about {:.3e} raw WAD per share \
         (dominated by one-share pools); max relative error on funded pools {:.3e}",
        f64_of(&max_err_raw),
        f64_of(&max_rel_err_funded)
    );
    assert!(cases > 300, "sweep too small: {cases}");
    assert!(
        max_rel_err_funded <= frac(big(2), pow10(9)),
        "LP fair value relative error on funded pools {:.3e} exceeds 2e-9",
        f64_of(&max_rel_err_funded)
    );
}

// ---------------------------------------------------------------------------
// Live controller fixtures
// ---------------------------------------------------------------------------

const USDC: &str = "USDC";
const USDT: &str = "USDT";
const ONE_USDC: i128 = 10_000_000; // 7 decimals

fn build() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .build()
}

fn key(t: &LendingTest, name: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(name))
}

fn try_borrow_raw(
    t: &mut LendingTest,
    user: &str,
    name: &str,
    amount: i128,
) -> Result<(), soroban_sdk::Error> {
    let account_id = t.resolve_account_id(user);
    let addr = t.get_or_create_user(user);
    let asset = t.resolve_asset(name);
    let ctrl = t.ctrl_client();
    let borrows = vec![&t.env, (hub_asset(asset), amount)];
    match ctrl.try_borrow(&addr, &account_id, &borrows, &None) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

/// `(scaled debt, projected borrow index)` of `user`'s `name` debt leg.
fn debt_leg(t: &LendingTest, user: &str, name: &str) -> (i128, i128) {
    let acc = t.resolve_account_id(user);
    let (_, debts) = t.ctrl_client().get_account_positions(&acc);
    let scaled = debts.get(key(t, name)).map_or(0, |p| p.scaled_amount);
    let index = t.ctrl_client().get_market_index(&key(t, name)).borrow_index;
    (scaled, index)
}

/// Ceiled debt in base units, as `unscale_borrow_ceil` computes it.
fn ceil_debt_units(env: &Env, scaled: i128, index: i128, decimals: u32) -> i128 {
    Ray::from(scaled)
        .mul_ceil(env, Ray::from(index))
        .to_asset_ceil(env, decimals)
}

#[test]
fn rv_round_9_gate_pass_is_never_liquidatable_and_parity_is_exact() {
    let mut t = build();
    let env = t.env.clone();
    // 1000 USDC at $1, LTV 75%, LT 80%: the LTV cap is 750 USDT exactly.
    t.supply_raw(ALICE, USDC, 1_000 * ONE_USDC);
    t.borrow_raw(ALICE, USDT, 750 * ONE_USDC);
    let acc = t.resolve_account_id(ALICE);

    // One more base unit of debt fails the LTV gate: D = 750.0000001e18 > ltv 750e18.
    assert_contract_error(
        try_borrow_raw(&mut t, ALICE, USDT, 1),
        errors::codes::INSUFFICIENT_COLLATERAL,
    );
    let hf = t.ctrl_client().get_health_factor(&acc);
    let w = t.ctrl_client().get_liquidation_collateral(&acc);
    let d = t.ctrl_client().get_total_borrow_usd(&acc);
    assert_eq!(w, 800 * WAD, "W = floor(1000 * 0.8)");
    assert_eq!(d, 750 * WAD, "D = 750 USDT at $1");
    assert_eq!(
        hf,
        mul_div_floor(&env, w, WAD, d),
        "HF = floor(W * WAD / D)"
    );
    assert!(
        hf >= WAD + WAD / 20,
        "max-LTV borrow leaves HF >= LT/LTV - slack"
    );
    assert!(!t.can_be_liquidated(ALICE));

    // Exact parity: W == D at USDC = $0.9375 (800 * 0.9375 = 750). HF is one WAD
    // exactly, so the account is NOT liquidatable: the floor of an exact ratio
    // does not shave a unit.
    t.set_price(USDC, 937_500_000_000_000_000);
    let w = t.ctrl_client().get_liquidation_collateral(&acc);
    let hf = t.ctrl_client().get_health_factor(&acc);
    assert_eq!(w, 750 * WAD, "W at parity");
    assert_eq!(hf, WAD, "HF is exactly one WAD at W == D");
    assert!(
        !t.can_be_liquidated(ALICE),
        "exact parity is not liquidatable"
    );

    // One Reflector tick (1e4 raw WAD, 14-decimal feed) below parity.
    t.set_price(USDC, 937_500_000_000_000_000 - 10_000);
    let w = t.ctrl_client().get_liquidation_collateral(&acc);
    let hf = t.ctrl_client().get_health_factor(&acc);
    assert_eq!(w, 750 * WAD - 8_000_000, "W = 800 * (P - 1e4)");
    assert_eq!(hf, mul_div_floor(&env, w, WAD, 750 * WAD));
    assert_eq!(hf, WAD - 10_667, "floor(1e18 - 10666.67)");
    assert!(t.can_be_liquidated(ALICE));
    println!("RV-R9 D: parity HF={WAD} not liquidatable; one tick below HF={hf} liquidatable");

    // After accrual the debt carries a fractional base unit. The HF view equals
    // floor(W * WAD / ceil(D)) bit for bit, with ceil(D) at most one raw WAD
    // above the half-up display (USDT at $1, 7 decimals: 1 base unit = 1e11 raw).
    t.set_price(USDC, WAD);
    t.advance_time(3_601);
    let (scaled, bi) = debt_leg(&t, ALICE, USDT);
    let d_ceil_wad = ceil_ray_to_wad(&env, Ray::from(scaled).mul_ceil(&env, Ray::from(bi)));
    let d_half = t.ctrl_client().get_total_borrow_usd(&acc);
    let w = t.ctrl_client().get_liquidation_collateral(&acc);
    let hf = t.ctrl_client().get_health_factor(&acc);
    assert!(
        d_ceil_wad >= d_half && d_ceil_wad - d_half <= 1,
        "ceil(D) - half_up(D) in {{0,1}} raw WAD"
    );
    assert_eq!(
        hf,
        mul_div_floor(&env, w, WAD, d_ceil_wad),
        "HF view mirrors floor(W*WAD/ceil(D))"
    );
    println!("RV-R9 D: after 3601 s D_half={d_half} D_ceil={d_ceil_wad} W={w} HF={hf}");
}

/// `Ray::to_wad_ceil` (crate-private in `common`): `ceil(ray / 1e9)`. The third
/// step, `ceil(x * price / WAD)`, is the identity at a $1 price.
fn ceil_ray_to_wad(env: &Env, ray: Ray) -> i128 {
    mul_div_ceil(env, ray.raw(), 1, 1_000_000_000)
}

#[test]
fn rv_round_9_min_borrow_floor_binds_at_the_exact_base_unit() {
    let mut t = build();
    let env = t.env.clone();
    assert_eq!(t.ctrl_client().get_min_borrow_collateral_usd(), 5 * WAD);

    // ltv = floor(V * 7500 / 10000). 66_666_667 base units ($6.6666667) gives
    // 5.000000025e18 >= 5e18; one base unit less gives 4.99999995e18 < 5e18.
    t.supply_raw(ALICE, USDC, 66_666_667);
    let alice = t.resolve_account_id(ALICE);
    let ltv = t.ctrl_client().get_ltv_collateral_usd(&alice);
    assert_eq!(
        ltv,
        mul_div_floor(&env, 6_666_666_700_000_000_000, 7_500, BPS)
    );
    assert_eq!(ltv, 5_000_000_025_000_000_000);
    t.borrow_raw(ALICE, USDT, 1);

    t.supply_raw(BOB, USDC, 66_666_666);
    let bob = t.resolve_account_id(BOB);
    let ltv = t.ctrl_client().get_ltv_collateral_usd(&bob);
    assert_eq!(ltv, 4_999_999_950_000_000_000);
    assert_contract_error(
        try_borrow_raw(&mut t, BOB, USDT, 1),
        errors::codes::MIN_BORROW_COLLATERAL_NOT_MET,
    );

    // A price with a 14th decimal makes the step-3 floor and the weight floor
    // visible: the view equals the exact rational floored twice and never exceeds it.
    t.set_price(USDC, 1_000_000_000_000_010_000);
    let v_exact = frac(big(66_666_667) * big(1_000_000_000_000_010_000), pow10(7));
    let ltv_exact = &v_exact * rat(7_500) / rat(BPS);
    let ltv = t.ctrl_client().get_ltv_collateral_usd(&alice);
    let v_floor = v_exact.floor().to_integer();
    let expected = (BigRational::from_integer(v_floor) * rat(7_500) / rat(BPS)).floor();
    assert_eq!(
        big(ltv),
        expected.to_integer(),
        "ltv = floor(floor(V) * LTV)"
    );
    assert!(
        rat(ltv) <= ltv_exact && &ltv_exact - rat(ltv) < rat(2),
        "two floors, under one raw WAD each"
    );
    println!(
        "RV-R9 E: floor binds at 66_666_667 vs 66_666_666 base units; ltv view {ltv} vs exact {}",
        f64_of(&ltv_exact)
    );
}

#[test]
fn rv_round_9_sub_display_dust_debt_keeps_the_five_dollar_floor_armed_until_one_more_unit() {
    let mut t = build();
    let env = t.env.clone();
    t.supply_raw(ALICE, USDC, 100 * ONE_USDC);
    t.borrow_raw(ALICE, USDT, 50 * ONE_USDC);
    let acc = t.resolve_account_id(ALICE);
    let usdt = key(&t, USDT);

    // Accrue until the debt's fractional base unit is below one half, so the
    // half-up display is one unit under the ceiled close amount.
    let mut found = None;
    for _ in 0..600 {
        t.advance_time(1);
        let (scaled, bi) = debt_leg(&t, ALICE, USDT);
        let shown = t.ctrl_client().get_borrow_amount(&acc, &usdt);
        let ceil = ceil_debt_units(&env, scaled, bi, 7);
        if shown < ceil {
            found = Some((shown, ceil, scaled, bi));
            break;
        }
    }
    let (shown, ceil, scaled, bi) = found.expect("a ledger where half-up < ceil");
    assert_eq!(
        ceil,
        shown + 1,
        "display and close amount differ by exactly one base unit"
    );

    // Repay exactly the displayed amount: the pool takes it as a partial repay
    // (amount < ceil), leaving a sub-unit scaled debt.
    t.repay_raw(ALICE, USDT, shown);
    let (dust_scaled, _) = debt_leg(&t, ALICE, USDT);
    assert!(
        dust_scaled > 0,
        "sub-unit debt remains: {dust_scaled} scaled"
    );
    assert_eq!(
        t.ctrl_client().get_borrow_amount(&acc, &usdt),
        0,
        "displayed debt is zero"
    );
    let d_half = t.ctrl_client().get_total_borrow_usd(&acc);
    let hf = t.ctrl_client().get_health_factor(&acc);
    assert!(
        d_half < WAD / 10_000_000,
        "debt displays below one base unit in USD: {d_half}"
    );
    assert!(hf > WAD, "healthy");
    assert!(!t.can_be_liquidated(ALICE));

    // Withdrawals that leave at least $5 of LTV collateral pass ...
    t.withdraw_raw(ALICE, USDC, 100 * ONE_USDC - 66_666_667);
    // ... one base unit more fails the min-borrow floor although the debt
    // displays as zero (the ceiled valuation keeps the account in debt).
    assert_contract_error(
        t.try_withdraw_raw(ALICE, USDC, 1),
        errors::codes::MIN_BORROW_COLLATERAL_NOT_MET,
    );

    // One more base unit closes the position (amount >= ceil(dust) == 1) with
    // no overpayment, and the collateral is free.
    let (dust_scaled, bi2) = debt_leg(&t, ALICE, USDT);
    assert_eq!(ceil_debt_units(&env, dust_scaled, bi2, 7), 1);
    t.repay_raw(ALICE, USDT, 1);
    let (closed, _) = debt_leg(&t, ALICE, USDT);
    assert_eq!(closed, 0, "debt leg closed");
    t.withdraw_raw(ALICE, USDC, 0);
    println!(
        "RV-R9 F: shown={shown} ceil={ceil} (scaled={scaled} bi={bi}); after repaying shown, dust \
         scaled={dust_scaled}, D_half={d_half} raw WAD; $5 floor blocked the withdrawal until 1 more unit"
    );
}

#[test]
fn rv_round_9_insolvency_branch_switches_exactly_at_c_equals_d() {
    let mut t = build();
    let env = t.env.clone();
    t.supply_raw(ALICE, USDC, 1_000 * ONE_USDC);
    t.borrow_raw(ALICE, USDT, 750 * ONE_USDC);
    let acc = t.resolve_account_id(ALICE);
    let usdt = key(&t, USDT);
    let offer = vec![&env, (usdt.clone(), 10_000 * ONE_USDC)];

    // C == D at USDC = $0.75 (1000 * 0.75 = 750): HF = 0.8, p = 0.8, cap = 0.
    // formulas.md: "a covered account takes the band quote with the cap clamped
    // to zero" -> (D, 0).
    t.set_price(USDC, 750_000_000_000_000_000);
    let c = t.ctrl_client().get_total_collateral_usd(&acc);
    let d = t.ctrl_client().get_total_borrow_usd(&acc);
    assert_eq!(c, d, "C == D");
    assert!(t.can_be_liquidated(ALICE));
    let e = t
        .ctrl_client()
        .get_liquidation_estimate(&acc, &offer, &SeizeMode::Transfer);
    assert_eq!(e.bonus_rate_bps, 0, "cap clamps to zero at parity");
    assert_eq!(e.max_payment_wad, d, "band quotes the full debt");

    // One tick below: C < D, insolvent branch -> (floor(C / (1 + base)), base).
    t.set_price(USDC, 750_000_000_000_000_000 - 10_000);
    let c = t.ctrl_client().get_total_collateral_usd(&acc);
    assert!(c < d);
    let e = t
        .ctrl_client()
        .get_liquidation_estimate(&acc, &offer, &SeizeMode::Transfer);
    let base = 500;
    let backed = mul_div_floor(&env, c, WAD, WAD + base * WAD / BPS);
    assert_eq!(
        e.bonus_rate_bps, base,
        "insolvent branch pays the base bonus"
    );
    // The estimate reports the kept repayment after the insolvent trim floors
    // each leg to whole debt-token units: at most one USDT base unit ($1e-7)
    // below `floor(C / (1 + base))`, never above it.
    let one_usdt_unit = WAD / ONE_USDC;
    assert!(
        e.max_payment_wad <= backed.min(d) && backed.min(d) - e.max_payment_wad < one_usdt_unit,
        "quote {} within one debt unit below min(D, floor(C/(1+base))) = {}",
        e.max_payment_wad,
        backed.min(d)
    );
    let socialized = d - e.max_payment_wad;
    println!(
        "RV-R9 G: at C==D quote=({d}, 0); one tick below C={c}: quote=({}, {base}) so {socialized} raw \
         WAD ({:.2} USD of a {:.0} USD debt) is left for socialization",
        e.max_payment_wad,
        socialized as f64 / WAD as f64,
        d as f64 / WAD as f64
    );
    // The window in which rounding (not the price) decides the branch is the
    // valuation slack: under one raw WAD per leg here, 1e-9 USD at the price cap.
    assert!(
        d - c == 10_000_000,
        "one tick of USDC on 1000 tokens is 1e7 raw WAD"
    );
}

// ---------------------------------------------------------------------------
// Decimal extremes on the live controller: 18 decimals at a sub-dollar price,
// 0 decimals at a six-figure price
// ---------------------------------------------------------------------------

const MEME: &str = "MEME18";
const DEAL: &str = "DEAL0";
const ONE_MEME: i128 = 1_000_000_000_000_000_000; // 18 decimals
const MEME_PRICE: i128 = WAD / 1_000; // $0.001: one base unit is worth 1e-21 USD
const DEAL_PRICE: i128 = 1_000_000 * WAD; // $1e6 per whole unit

/// An 18-decimal token at $0.001 (one base unit is worth 1e-3 raw WAD).
fn meme_preset() -> MarketPreset {
    MarketPreset {
        name: MEME,
        decimals: 18,
        price_wad: MEME_PRICE,
        initial_liquidity: 10_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// A 0-decimal, collateral-only token at $1e6 per unit (sub-3-decimal rules:
/// not borrowable, no liquidation fee, no flash loans).
fn deal_preset() -> MarketPreset {
    MarketPreset {
        name: DEAL,
        decimals: 0,
        price_wad: DEAL_PRICE,
        initial_liquidity: 0.0,
        config: AssetConfigPreset {
            is_borrowable: false,
            is_flashloanable: false,
            flashloan_fee: 0,
            liquidation_fees: 0,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    }
}

#[test]
fn rv_round_9_eighteen_decimal_dust_debt_is_one_raw_wad_and_parity_is_exact_at_the_feed_tick() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(meme_preset())
        .build();
    let env = t.env.clone();

    // ALICE: $100 USDC collateral (ltv $75, W $80), one base unit of MEME18 debt.
    t.supply_raw(ALICE, USDC, 100 * ONE_USDC);
    t.borrow_raw(ALICE, MEME, 1);
    let alice = t.resolve_account_id(ALICE);
    let meme = key(&t, MEME);

    // Pool: scaled = ceil(1e9 * RAY / RAY) = 1e9 raw RAY (1e-18 whole tokens).
    let (scaled, bi) = debt_leg(&t, ALICE, MEME);
    assert_eq!(bi, RAY, "fresh market: borrow index is one RAY");
    assert_eq!(
        scaled, 1_000_000_000,
        "one base unit of an 18-decimal token in RAY"
    );

    // Risk debt: ceil at every step -> mul_ceil 1e9, to_wad_ceil 1, mul_ceil(1 * 1e15 / 1e18) = 1.
    // Display: half-up at every step -> 0. The base-unit display is 1.
    let d_ceil = position_value_ceil(
        &env,
        Ray::from(scaled),
        Ray::from(bi),
        Wad::from(MEME_PRICE),
    )
    .raw();
    let d_half = t.ctrl_client().get_total_borrow_usd(&alice);
    assert_eq!(d_ceil, 1, "a positive debt never values to zero for risk");
    assert_eq!(d_half, 0, "the half-up display of a 1e-21 USD debt is zero");
    assert_eq!(t.ctrl_client().get_borrow_amount(&alice, &meme), 1);

    // HF = floor(W * WAD / 1) = 80e18 * 1e18 = 8e37: finite (below i128::MAX), healthy.
    let hf = t.ctrl_client().get_health_factor(&alice);
    let w = t.ctrl_client().get_liquidation_collateral(&alice);
    assert_eq!(w, 80 * WAD);
    assert_eq!(hf, mul_div_floor(&env, w, WAD, d_ceil));
    assert_eq!(hf, 80 * WAD * WAD);
    assert!(hf < i128::MAX, "no saturation at 8e37");
    assert!(!t.can_be_liquidated(ALICE));

    // The account is in debt: the $5 LTV floor gates withdrawals although the
    // debt is worth 1e-21 USD. 66_666_667 USDC base units keep ltv = 5.000000025e18.
    t.withdraw_raw(ALICE, USDC, 100 * ONE_USDC - 66_666_667);
    assert_contract_error(
        t.try_withdraw_raw(ALICE, USDC, 1),
        errors::codes::MIN_BORROW_COLLATERAL_NOT_MET,
    );
    // Repaying the single base unit closes the leg (ceil(dust) == 1) and frees it.
    t.repay_raw(ALICE, MEME, 1);
    assert_eq!(debt_leg(&t, ALICE, MEME).0, 0);
    t.withdraw_raw(ALICE, USDC, 0);
    println!(
        "RV-R9 H1: 1 base unit of MEME18 at $0.001: scaled={scaled} D_ceil={d_ceil} D_half={d_half} HF={hf}"
    );

    // BOB: 10_000 MEME18 ($10) as collateral, 7.5 USDT debt: the LTV cap exactly.
    t.supply_raw(BOB, MEME, 10_000 * ONE_MEME);
    t.borrow_raw(BOB, USDT, 75_000_000);
    let bob = t.resolve_account_id(BOB);
    assert_eq!(
        t.ctrl_client().get_ltv_collateral_usd(&bob),
        7_500_000_000_000_000_000
    );
    assert_contract_error(
        try_borrow_raw(&mut t, BOB, USDT, 1),
        errors::codes::INSUFFICIENT_COLLATERAL,
    );

    // Exact parity: V = 1e22 * P / 1e18 = 9.375e18 at P = 9.375e14 (a 14-decimal
    // feed value), W = floor(0.8 * V) = 7.5e18 == D. HF is one WAD: not liquidatable.
    let parity = 937_500_000_000_000;
    t.set_price(MEME, parity);
    assert_eq!(
        t.ctrl_client().get_total_collateral_usd(&bob),
        9_375_000_000_000_000_000
    );
    assert_eq!(
        t.ctrl_client().get_liquidation_collateral(&bob),
        7_500_000_000_000_000_000
    );
    assert_eq!(t.ctrl_client().get_health_factor(&bob), WAD);
    assert!(!t.can_be_liquidated(BOB));

    // One feed tick (1e4 raw WAD) below: V = 9.375e18 - 1e8, W = 7.5e18 - 8e7,
    // HF = floor(1e18 - 10_666_666.67) = 1e18 - 10_666_667: liquidatable.
    t.set_price(MEME, parity - 10_000);
    let w = t.ctrl_client().get_liquidation_collateral(&bob);
    let hf = t.ctrl_client().get_health_factor(&bob);
    assert_eq!(w, 7_500_000_000_000_000_000 - 80_000_000);
    assert_eq!(hf, WAD - 10_666_667);
    assert!(t.can_be_liquidated(BOB));
    println!("RV-R9 H2: MEME18 parity HF={WAD} not liquidatable; one tick below W={w} HF={hf}");
}

#[test]
fn rv_round_9_zero_decimal_parity_floor_is_exact_and_whole_unit_floor_binds() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(MarketPreset {
            initial_liquidity: 5_000_000.0,
            ..usdt_stable_preset()
        })
        .with_market(deal_preset())
        .build();
    let env = t.env.clone();

    // CAROL: 2 units of DEAL0 at $1e6 (sole supply leg, the whole-unit minimum),
    // V = 2e24 raw WAD exactly, ltv = 1.5e24, W = 1.6e24. Borrow 1.5e6 USDT.
    t.supply_raw(BOB, DEAL, 2);
    let carol = t.resolve_account_id(BOB);
    t.borrow_raw(BOB, USDT, 1_500_000 * ONE_USDC);
    let v = t.ctrl_client().get_total_collateral_usd(&carol);
    let ltv = t.ctrl_client().get_ltv_collateral_usd(&carol);
    let w = t.ctrl_client().get_liquidation_collateral(&carol);
    let d = t.ctrl_client().get_total_borrow_usd(&carol);
    assert_eq!(v, 2 * DEAL_PRICE);
    assert_eq!(ltv, 1_500_000 * WAD);
    assert_eq!(w, 1_600_000 * WAD);
    assert_eq!(d, 1_500_000 * WAD);
    assert_eq!(
        t.ctrl_client().get_health_factor(&carol),
        mul_div_floor(&env, w, WAD, d)
    );
    assert_eq!(
        t.ctrl_client().get_health_factor(&carol),
        1_066_666_666_666_666_666
    );
    assert_contract_error(
        try_borrow_raw(&mut t, BOB, USDT, 1),
        errors::codes::INSUFFICIENT_COLLATERAL,
    );

    // Exact parity at P = $937_500: W = floor(0.8 * 2P) = 1.5e24 == D, HF one WAD.
    let parity = 937_500 * WAD;
    t.set_price(DEAL, parity);
    assert_eq!(
        t.ctrl_client().get_liquidation_collateral(&carol),
        1_500_000 * WAD
    );
    assert_eq!(t.ctrl_client().get_health_factor(&carol), WAD);
    assert!(!t.can_be_liquidated(BOB));

    // One feed tick below: W = 1.5e24 - 16_000 raw WAD. The exact HF is
    // 1 - 1.07e-20, i.e. below one by less than one raw WAD; the floor returns
    // 1e18 - 1 and the account is liquidatable, as the integers W < D require.
    t.set_price(DEAL, parity - 10_000);
    let w = t.ctrl_client().get_liquidation_collateral(&carol);
    let hf = t.ctrl_client().get_health_factor(&carol);
    assert_eq!(w, 1_500_000 * WAD - 16_000);
    assert_eq!(hf, WAD - 1, "floor(1e18 - 0.0107) = 1e18 - 1");
    assert!(t.can_be_liquidated(BOB));
    let exact_hf = frac(big(w), big(1_500_000 * WAD));
    assert!(
        exact_hf < rat(1),
        "the exact ratio is below one as well: no misclassification"
    );
    println!(
        "RV-R9 I1: DEAL0 parity HF={WAD}; one tick below W={w} HF={hf} (exact 1 - {:.3e})",
        f64_of(&(rat(1) - exact_hf))
    );

    // Whole-unit floor: leave one base unit of USDT debt, then withdrawing one
    // DEAL0 unit would leave 1 whole unit < MIN_WHOLE_UNIT_COLLATERAL (2) while
    // in debt. The LTV ($750k), HF and $5 gates all pass; the whole-unit gate fails.
    t.set_price(DEAL, DEAL_PRICE);
    t.repay_raw(BOB, USDT, 1_500_000 * ONE_USDC - 1);
    assert_eq!(t.ctrl_client().get_total_borrow_usd(&carol), WAD / ONE_USDC);
    assert_contract_error(
        t.try_withdraw_raw(BOB, DEAL, 1),
        errors::codes::MIN_BORROW_COLLATERAL_NOT_MET,
    );
    // Repay the last unit; the whole-unit floor no longer applies and 1 unit can leave.
    t.repay_raw(BOB, USDT, 1);
    t.withdraw_raw(BOB, DEAL, 1);
    assert_eq!(t.ctrl_client().get_total_collateral_usd(&carol), DEAL_PRICE);
    println!(
        "RV-R9 I2: whole-unit floor blocked 2 -> 1 units while 1 USDT base unit of debt remained"
    );
}
