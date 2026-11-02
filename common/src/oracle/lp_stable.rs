//! Fair-value pricing for two-coin StableSwap-style LP pools, solving the
//! invariant `D` via Newton-Raphson iteration on WAD-normalized reserves.

use crate::errors::OracleError;
use crate::oracle::lp::{LpLeg, LpSupply};
use crate::oracle::observation::{try_amount_to_wad, try_u256_to_i128};
use soroban_sdk::{Env, U256};

/// Number of coins the invariant computations in this module support.
const N_COINS: u32 = 2;

/// Inclusive bounds on the amplification coefficient accepted by
/// [`solve_stable_d`].
const MIN_AMP: u128 = 1;
const MAX_AMP: u128 = 1_000_000;

/// Maximum Newton-Raphson iterations attempted when solving for `D` before
/// [`solve_stable_d`] returns an error.
const MAX_D_ITERATIONS: u32 = 255;

/// Maximum WAD-normalized reserve accepted per leg by [`solve_stable_d`].
const MAX_NORMALIZED_RESERVE_WAD: u128 = 10u128.pow(34);

/// Solves the two-coin StableSwap invariant `D` for WAD-normalized reserves
/// `xa_wad` and `xb_wad` at amplification `amp`, via Newton-Raphson
/// iteration. The result does not depend on the order of the two reserves.
/// Returns `OracleError::InvalidPrice` if either reserve is not positive,
/// `amp` is outside `[MIN_AMP, MAX_AMP]`, either reserve exceeds
/// `MAX_NORMALIZED_RESERVE_WAD`, an intermediate product overflows `U256`
/// (extreme reserve imbalance), or the iteration does not converge within
/// `MAX_D_ITERATIONS` steps.
fn solve_stable_d(env: &Env, xa_wad: i128, xb_wad: i128, amp: u128) -> Result<U256, OracleError> {
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
    // `d_p` divides by one reserve and then the other. The first quotient's
    // truncation is multiplied by `d / (2 * second reserve)`, so dividing by the
    // larger reserve first lets it outgrow the exit test below and the
    // iteration cycles. Dividing by the smaller reserve first keeps it at a
    // unit or two.
    let x_smaller = U256::from_u128(env, xa_wad.min(xb_wad) as u128);
    let x_larger = U256::from_u128(env, xa_wad.max(xb_wad) as u128);

    let sum = x_smaller.add(&x_larger);
    let ann = U256::from_u128(env, amp).mul(&n);

    // Every iterate is at most 2 * sum (< 2^116), so `d * d`, `ann * sum` and
    // `(ann - 1) * d` fit. A fitting `d_p * d` bounds `d_p` below 2^254 once
    // x_larger >= 2 (x_larger = 1 keeps every value tiny), so the sums and
    // `3 * d_p` fit: only `d_p * d` and the numerator's `* d` can exceed U256.
    let mut d = sum.clone();
    for _ in 0..MAX_D_ITERATIONS {
        let d_p = d.mul(&d).div(&x_smaller.mul(&n));
        let d_p = checked_mul(&d_p, &d)?.div(&x_larger.mul(&n));

        let d_prev = d.clone();
        let numerator = checked_mul(&ann.mul(&sum).add(&n.mul(&d_p)), &d)?;
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

/// `a * b`, or `InvalidPrice` where `U256` would overflow and trap the host.
fn checked_mul(a: &U256, b: &U256) -> Result<U256, OracleError> {
    a.checked_mul(b).ok_or(OracleError::InvalidPrice)
}

/// Computes the fair-value price of one LP share, in WAD (1e18) scale, for a
/// two-coin StableSwap pool.
///
/// Converts both reserves to WAD, solves the invariant `D` via
/// [`solve_stable_d`], multiplies `D` by the lower of the two legs'
/// WAD-scaled prices, and divides by the share supply converted to WAD.
/// Returns `OracleError::InvalidPrice` if any reserve, price, or share
/// amount is not positive, if `D` fails to solve, or if the result does not
/// fit in `i128`.
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

    let xa_wad = try_amount_to_wad(env, a.reserve, a.decimals)?;
    let xb_wad = try_amount_to_wad(env, b.reserve, b.decimals)?;
    let d = solve_stable_d(env, xa_wad, xb_wad, amp)?;

    let min_price = a.price_wad.min(b.price_wad);
    let share_supply_wad = try_amount_to_wad(env, supply.total_shares, supply.decimals)?;
    if share_supply_wad <= 0 {
        return Err(OracleError::InvalidPrice);
    }

    let fair = d
        .mul(&U256::from_u128(env, min_price as u128))
        .div(&U256::from_u128(env, share_supply_wad as u128));
    try_u256_to_i128(&fair).ok_or(OracleError::InvalidPrice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::WAD;
    use crate::oracle::lp::isqrt_of_product;

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

    fn solve_stable_y(env: &Env, x_wad: i128, d: &U256, amp: u128) -> U256 {
        let n = U256::from_u32(env, N_COINS);
        let one = U256::from_u32(env, 1);
        let x = U256::from_u128(env, x_wad as u128);
        let ann = U256::from_u128(env, amp).mul(&n);

        let mut c = d.mul(d).div(&x.mul(&n));
        c = c.mul(d).div(&ann.mul(&n));
        let b = x.add(&d.div(&ann));

        let mut y = d.clone();
        for _ in 0..MAX_D_ITERATIONS {
            let y_prev = y.clone();
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
        assert_eq!(
            price / 100_000_000_000,
            10_012_514,
            "fair price must floor to the pool's virtual_price 1.0012514: {price}"
        );
    }

    #[test]
    fn swap_cannot_move_the_price() {
        let env = Env::default();
        let (ra, rb, amp) = (40_701_828_372_545i128, 39_592_003_957_960i128, 1500u128);
        let s = supply(80_193_465_649_977);

        let before =
            fair_stable_lp_price_wad(&env, &leg(ra, 7, WAD), &leg(rb, 7, WAD), &s, amp).unwrap();

        let d = solve_stable_d(&env, ra * 100_000_000_000, rb * 100_000_000_000, amp).unwrap();
        let ra2_wad = (ra + ra / 2) * 100_000_000_000;
        let rb2_wad = try_u256_to_i128(&solve_stable_y(&env, ra2_wad, &d, amp)).unwrap();
        let ra2 = ra + ra / 2;
        let rb2 = rb2_wad / 100_000_000_000;

        let after =
            fair_stable_lp_price_wad(&env, &leg(ra2, 7, WAD), &leg(rb2, 7, WAD), &s, amp).unwrap();

        let drift = (before - after).abs();
        assert!(drift < 1_000_000_000, "swap moved price by {drift} wad");
    }

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

    /// Covers the reserve cap and, at 10^15 : 10^3 tokens, a pool whose first
    /// Newton step overflows `U256`, which must not trap the host.
    #[test]
    fn absurd_reserve_errors_not_panics() {
        let env = Env::default();
        for (reserve_a, reserve_b, amp) in [
            (i128::MAX, 1_000_000_000, 1500),
            (10i128.pow(22), 10i128.pow(10), 100),
        ] {
            let err = fair_stable_lp_price_wad(
                &env,
                &leg(reserve_a, 7, WAD),
                &leg(reserve_b, 7, WAD),
                &supply(1_000_000_000),
                amp,
            )
            .unwrap_err();
            assert_eq!(err, OracleError::InvalidPrice);
        }
        let (big, small) = (10i128.pow(33), 10i128.pow(21));
        for (xa, xb) in [(big, small), (small, big)] {
            assert_eq!(
                solve_stable_d(&env, xa, xb, 100),
                Err(OracleError::InvalidPrice)
            );
        }
    }

    /// Exact `floor(D)` of the 10^10 : 10^7 token pool at amp 2.
    const TWO_CYCLE_ROOT: u128 = 2_362_217_299_922_534_376_201_222_040;

    /// Dividing by the larger reserve first made this pool's iterates cycle
    /// between root + 1 and root + 3 until the iteration cap.
    #[test]
    fn imbalanced_pool_solves_exactly_in_either_leg_order() {
        let env = Env::default();
        let (big, small) = (10_000_000_000 * WAD, 10_000_000 * WAD);
        for (xa, xb) in [(big, small), (small, big)] {
            assert_eq!(
                solve_stable_d(&env, xa, xb, 2).map(|d| d.to_u128()),
                Ok(Some(TWO_CYCLE_ROOT))
            );
        }
    }

    /// Admitted pools from 500:1 to 1,000,000:1 solve inside the invariant's
    /// bounds `[2 sqrt(xa xb), xa + xb]`, to the same `D` in either leg order.
    #[test]
    fn imbalanced_pool_grid_solves_symmetrically() {
        let env = Env::default();
        for amp in [1u128, 2, 5, 10, 100, 1_000] {
            for ratio in [500i128, 1_000, 10_000, 1_000_000] {
                for exp in 3..=11u32 {
                    let big = 10i128.pow(exp) * WAD;
                    let small = big / ratio;
                    // One solve per default budget, as in a real invocation.
                    env.cost_estimate().budget().reset_default();
                    let d = solve_stable_d(&env, big, small, amp)
                        .unwrap_or_else(|e| panic!("{big}:{small} amp {amp}: {e:?}"));
                    assert_eq!(solve_stable_d(&env, small, big, amp), Ok(d.clone()));

                    let d = d.to_u128().unwrap();
                    let lower = 2 * isqrt_of_product(&env, big as u128, small as u128)
                        .to_u128()
                        .unwrap();
                    assert!(
                        d + 2 >= lower && d <= (big + small) as u128,
                        "{big}:{small} amp {amp}: D {d} out of bounds"
                    );
                }
            }
        }
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
