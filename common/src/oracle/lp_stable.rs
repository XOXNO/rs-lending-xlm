//! Manipulation-resistant fair-value pricing for stableswap (Curve-style) AMM
//! LP shares.
//!
//! A stableswap pool holds near-pegged assets on the invariant
//! `A·nⁿ·Σxᵢ + D = A·D·nⁿ + Dⁿ⁺¹/(nⁿ·Πxᵢ)`. `D` — the invariant "size" — is
//! preserved by swaps (only deposits, withdrawals, fees and the `A`-ramp move
//! it), exactly as `k = xy` is for constant-product. Pricing off `D` therefore
//! cancels the reserve *split*, the only thing a flash-loan swap can move.
//!
//! Value is `(D / S) · min(Pᵢ)`: `D/S` is the invariant units per share and
//! `min(Pᵢ)` the conservative USD floor per unit — under a depeg the pool
//! drains into the cheapest leg, so the cheapest oracle price is what an LP
//! redeemer actually recovers. Both legs' prices come from independent oracles.
//!
//! Every anomaly (non-positive input, out-of-range `A`, absurd magnitude, or a
//! `D` iteration that fails to converge) maps to an error, never a panic: a
//! compromised pool must not brick the market with an unrecoverable host trap,
//! and must never yield a manipulable number.

use crate::constants::WAD_DECIMALS;
use crate::errors::OracleError;
use crate::math::fp_core::try_mul_div_half_up;
use crate::oracle::lp::{LpLeg, LpSupply};
use crate::oracle::observation::try_u256_to_i128;
use soroban_sdk::{Env, U256};

/// Two-coin pool. Aquarius constant-product and stableswap pools are both pairs.
const N_COINS: u32 = 2;

/// `A` bounds. Curve caps amplification at 1e6; below 1 the invariant degrades
/// to (and past) constant-sum. An `A` outside this is a compromised/garbage row.
const MIN_AMP: u128 = 1;
const MAX_AMP: u128 = 1_000_000;

/// The `D` Newton iteration converges in a handful of steps for sane pools; the
/// cap is a safety backstop. Non-convergence within it is treated as an error.
const MAX_D_ITERATIONS: u32 = 255;

/// Upper bound on a WAD-normalized reserve. Keeps every `U256` product in the
/// `D` iteration (`~Ann·Σx·D`) well inside 256 bits, so the solver can never
/// overflow-panic. 1e34 WAD is a ~$1e16 leg — astronomically above any real
/// pool, so a larger value is itself evidence of a corrupt reserve.
const MAX_NORMALIZED_RESERVE_WAD: u128 = 10u128.pow(34);

/// Manipulation-resistant invariant `D` for a two-coin stableswap, over reserves
/// already normalized to a common WAD scale.
///
/// `D` is swap-invariant, so pricing off it is immune to reserve-split
/// manipulation. Computed by Newton's method in `U256`; the per-coin division
/// keeps `D_P` near `D`'s magnitude (never forming `D³` directly).
///
/// # Errors
/// * [`OracleError::InvalidPrice`] - non-positive reserve, `amp` out of range,
///   a reserve past [`MAX_NORMALIZED_RESERVE_WAD`], or non-convergence.
pub fn solve_stable_d(
    env: &Env,
    xa_wad: i128,
    xb_wad: i128,
    amp: u128,
) -> Result<U256, OracleError> {
    if xa_wad <= 0
        || xb_wad <= 0
        || !(MIN_AMP..=MAX_AMP).contains(&amp)
        || xa_wad as u128 > MAX_NORMALIZED_RESERVE_WAD
        || xb_wad as u128 > MAX_NORMALIZED_RESERVE_WAD
    {
        return Err(OracleError::InvalidPrice);
    }

    let n = U256::from_u32(env, N_COINS);
    let one = U256::from_u32(env, 1);
    let xa = U256::from_u128(env, xa_wad as u128);
    let xb = U256::from_u128(env, xb_wad as u128);

    let sum = xa.add(&xb);
    // Ann = A · n. Aquarius' `a()` returns the Curve *code* amplification (which
    // already folds in the whitepaper's n^(n-1) factor), so `get_D` multiplies it
    // by n, not nⁿ. Using nⁿ would model ~2x the pool's real amplification, over-
    // valuing shares under imbalance and breaking swap-invariance vs the pool.
    let ann = U256::from_u128(env, amp).mul(&n);

    let mut d = sum.clone();
    for _ in 0..MAX_D_ITERATIONS {
        // D_P = Dⁿ⁺¹ / (nⁿ · Πxᵢ), accumulated one coin at a time so the running
        // value stays near D's magnitude instead of forming Dⁿ⁺¹ outright.
        let mut d_p = d.clone();
        d_p = d_p.mul(&d).div(&xa.mul(&n));
        d_p = d_p.mul(&d).div(&xb.mul(&n));

        let d_prev = d.clone();
        // D = (Ann·S + n·D_P)·D / ((Ann-1)·D + (n+1)·D_P)
        let numerator = ann.mul(&sum).add(&n.mul(&d_p)).mul(&d);
        let denominator = ann.sub(&one).mul(&d).add(&n.add(&one).mul(&d_p));
        d = numerator.div(&denominator);

        let converged = if d >= d_prev {
            d.sub(&d_prev) <= one
        } else {
            d_prev.sub(&d) <= one
        };
        if converged {
            return Ok(d);
        }
    }
    Err(OracleError::InvalidPrice)
}

/// Fair USD price (WAD) of one whole LP share of a two-coin stableswap:
/// `D · min(Pₐ, P_b) / S_whole`, with `D` the invariant over WAD-normalized
/// reserves. The two `WAD` factors (unit→USD and per-share) cancel to a single
/// `U256` `mul`/`div`.
///
/// # Errors
/// * [`OracleError::InvalidPrice`] - non-positive reserve/price/supply, a
///   decimals value past [`WAD_DECIMALS`], overflow, or `D` non-convergence.
pub fn fair_stable_lp_price_wad(
    env: &Env,
    a: &LpLeg,
    b: &LpLeg,
    supply: &LpSupply,
    amp: u128,
) -> Result<i128, OracleError> {
    if a.reserve <= 0
        || b.reserve <= 0
        || a.price_wad <= 0
        || b.price_wad <= 0
        || supply.total_shares <= 0
    {
        return Err(OracleError::InvalidPrice);
    }

    let xa_wad = amount_to_wad(env, a.reserve, a.decimals)?;
    let xb_wad = amount_to_wad(env, b.reserve, b.decimals)?;
    let d = solve_stable_d(env, xa_wad, xb_wad, amp)?;

    // Conservative unit price: the pool converges into whichever leg is cheaper.
    let min_price = a.price_wad.min(b.price_wad);
    let share_supply_wad = amount_to_wad(env, supply.total_shares, supply.decimals)?;
    if share_supply_wad <= 0 {
        return Err(OracleError::InvalidPrice);
    }

    let fair = d
        .mul(&U256::from_u128(env, min_price as u128))
        .div(&U256::from_u128(env, share_supply_wad as u128));
    try_u256_to_i128(&fair).ok_or(OracleError::InvalidPrice)
}

/// Normalized invariant `D` (WAD) for a two-coin stableswap from raw reserves.
/// Exposed for the listing-time cross-check between the pool and its plane
/// mirror: `D` is the swap-invariant the two views must agree on.
///
/// # Errors
/// * [`OracleError::InvalidPrice`] - as [`solve_stable_d`], or a `D` past `i128`.
pub fn stable_invariant_d_wad(
    env: &Env,
    reserve_a: i128,
    decimals_a: u32,
    reserve_b: i128,
    decimals_b: u32,
    amp: u128,
) -> Result<i128, OracleError> {
    let xa = amount_to_wad(env, reserve_a, decimals_a)?;
    let xb = amount_to_wad(env, reserve_b, decimals_b)?;
    let d = solve_stable_d(env, xa, xb, amp)?;
    try_u256_to_i128(&d).ok_or(OracleError::InvalidPrice)
}

/// `amount` (in `decimals`) upscaled to WAD; `InvalidPrice` on overflow or
/// `decimals > 18`.
fn amount_to_wad(env: &Env, amount: i128, decimals: u32) -> Result<i128, OracleError> {
    let scale = WAD_DECIMALS
        .checked_sub(decimals)
        .and_then(|exp| 10i128.checked_pow(exp))
        .ok_or(OracleError::InvalidPrice)?;
    try_mul_div_half_up(env, amount, scale, 1).ok_or(OracleError::InvalidPrice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::WAD;

    fn leg(reserve: i128, decimals: u32, price_wad: i128) -> LpLeg {
        LpLeg {
            reserve,
            decimals,
            price_wad,
        }
    }

    fn supply(total_shares: i128) -> LpSupply {
        LpSupply {
            total_shares,
            decimals: 7,
        }
    }

    /// Reference `get_y`: the reserve `y` for a target `x` on invariant `D` — a
    /// swap moving the pool from `(x0, y0)` to `(x, y)`. Same math as the pool's
    /// own swap, so post-swap reserves lie exactly on invariant `D`.
    fn solve_stable_y(env: &Env, x_wad: i128, d: &U256, amp: u128) -> U256 {
        let n = U256::from_u32(env, N_COINS);
        let one = U256::from_u32(env, 1);
        let x = U256::from_u128(env, x_wad as u128);
        let ann = U256::from_u128(env, amp).mul(&n);

        // c = D^(n+1) / (nⁿ · x · Ann); b = x + D/Ann
        let mut c = d.mul(d).div(&x.mul(&n));
        c = c.mul(d).div(&ann.mul(&n));
        let b = x.add(&d.div(&ann));

        let mut y = d.clone();
        for _ in 0..MAX_D_ITERATIONS {
            let y_prev = y.clone();
            // y = (y² + c) / (2y + b - D)
            let numerator = y.mul(&y).add(&c);
            let denominator = n.mul(&y).add(&b).sub(d);
            y = numerator.div(&denominator);
            let done = if y >= y_prev {
                y.sub(&y_prev) <= one
            } else {
                y_prev.sub(&y) <= one
            };
            if done {
                break;
            }
        }
        y
    }

    // Real mainnet PYUSD/USDC Aquarius stableswap snapshot: reserves
    // 40701828372545 / 39592003957960 (7 dp), supply 80193465649977 (7 dp),
    // A = 1500. The pool's own get_virtual_price reads 1.0012514, so at both
    // legs $1.00 the fair share must land on ~$1.00125.
    #[test]
    fn fair_price_matches_reference_snapshot() {
        let env = Env::default();
        let price = fair_stable_lp_price_wad(
            &env,
            &leg(40_701_828_372_545, 7, WAD),
            &leg(39_592_003_957_960, 7, WAD),
            &supply(80_193_465_649_977),
            1500,
        )
        .unwrap();
        // Both legs at exactly $1.00, so fair = D/S. Pin the amplification
        // convention: floored to the pool's 7-dp scale it must equal the pool's
        // own get_virtual_price = 1.0012514. The wrong Ann = A·nⁿ yields
        // 1.0012515 here and fails this, which the old ±5e-4 window let through.
        assert_eq!(
            price / 100_000_000_000,
            10_012_514,
            "fair price must floor to the pool's virtual_price 1.0012514: {price}"
        );
    }

    // The core property: a swap preserves D, so the fair price must not move.
    // Start from the real pool, swap a large amount (imbalancing the split),
    // land the counter-reserve exactly on the same D via get_y, and assert the
    // price is unchanged. This is the flash-loan manipulation an attacker runs.
    #[test]
    fn swap_cannot_move_the_price() {
        let env = Env::default();
        let (ra, rb, amp) = (40_701_828_372_545i128, 39_592_003_957_960i128, 1500u128);
        let s = supply(80_193_465_649_977);

        let before =
            fair_stable_lp_price_wad(&env, &leg(ra, 7, WAD), &leg(rb, 7, WAD), &s, amp).unwrap();

        // Attacker dumps ~50% more of coin A into the pool; coin B falls to the
        // y that keeps invariant D fixed.
        let d = solve_stable_d(&env, ra * 100_000_000_000, rb * 100_000_000_000, amp).unwrap();
        let ra2_wad = (ra + ra / 2) * 100_000_000_000;
        let rb2_wad = try_u256_to_i128(&solve_stable_y(&env, ra2_wad, &d, amp)).unwrap();
        let ra2 = ra + ra / 2;
        let rb2 = rb2_wad / 100_000_000_000;

        let after =
            fair_stable_lp_price_wad(&env, &leg(ra2, 7, WAD), &leg(rb2, 7, WAD), &s, amp).unwrap();

        // Equal to within rounding dust (sub-1e-6 of a cent).
        let drift = (before - after).abs();
        assert!(drift < 1_000_000_000, "swap moved price by {drift} wad");
    }

    // A depeg on one leg must mark the whole share down to the cheaper price:
    // min(), not an average. Coin B at $0.90 → share ~10% cheaper, not ~5%.
    #[test]
    fn depeg_marks_to_the_cheaper_leg() {
        let env = Env::default();
        let (ra, rb) = (40_701_828_372_545i128, 39_592_003_957_960i128);
        let s = supply(80_193_465_649_977);
        let pegged =
            fair_stable_lp_price_wad(&env, &leg(ra, 7, WAD), &leg(rb, 7, WAD), &s, 1500).unwrap();
        let depegged = fair_stable_lp_price_wad(
            &env,
            &leg(ra, 7, WAD),
            &leg(rb, 7, 900_000_000_000_000_000),
            &s,
            1500,
        )
        .unwrap();
        // ~10% markdown (min price), decisively more than a 5% average would give.
        let ratio = depegged * 10_000 / pegged;
        assert!(
            (8_900..=9_100).contains(&ratio),
            "expected ~0.90x, got {ratio} bps"
        );
    }

    #[test]
    fn out_of_range_amp_errors() {
        let env = Env::default();
        for amp in [0u128, MAX_AMP + 1] {
            let err = fair_stable_lp_price_wad(
                &env,
                &leg(1_000_000_000, 7, WAD),
                &leg(1_000_000_000, 7, WAD),
                &supply(1_000_000_000),
                amp,
            )
            .unwrap_err();
            assert_eq!(err, OracleError::InvalidPrice);
        }
    }

    // A pathological reserve (compromised plane) must error, never panic.
    #[test]
    fn absurd_reserve_errors_not_panics() {
        let env = Env::default();
        let err = fair_stable_lp_price_wad(
            &env,
            &leg(i128::MAX, 7, WAD),
            &leg(1_000_000_000, 7, WAD),
            &supply(1_000_000_000),
            1500,
        )
        .unwrap_err();
        assert_eq!(err, OracleError::InvalidPrice);
    }

    #[test]
    fn rejects_zero_supply_and_reserve() {
        let env = Env::default();
        assert_eq!(
            fair_stable_lp_price_wad(&env, &leg(1, 7, WAD), &leg(1, 7, WAD), &supply(0), 1500)
                .unwrap_err(),
            OracleError::InvalidPrice
        );
        assert_eq!(
            fair_stable_lp_price_wad(
                &env,
                &leg(0, 7, WAD),
                &leg(1, 7, WAD),
                &supply(1_000_000_000),
                1500
            )
            .unwrap_err(),
            OracleError::InvalidPrice
        );
    }

    // A balanced pool at $1 prices at ~$1/share regardless of A.
    #[test]
    fn balanced_pool_prices_near_unit() {
        let env = Env::default();
        for amp in [1u128, 100, 1500, MAX_AMP] {
            let price = fair_stable_lp_price_wad(
                &env,
                &leg(1_000_000_000, 7, WAD),
                &leg(1_000_000_000, 7, WAD),
                &supply(2_000_000_000),
                amp,
            )
            .unwrap();
            let drift = (price - WAD).abs();
            assert!(drift < 1_000_000_000_000, "amp {amp}: {price}");
        }
    }
}
