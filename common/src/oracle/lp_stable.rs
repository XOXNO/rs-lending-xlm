use crate::constants::WAD_DECIMALS;
use crate::errors::OracleError;
use crate::math::fp_core::try_mul_div_half_up;
use crate::oracle::lp::{LpLeg, LpSupply};
use crate::oracle::observation::try_u256_to_i128;
use soroban_sdk::{Env, U256};

const N_COINS: u32 = 2;

const MIN_AMP: u128 = 1;
const MAX_AMP: u128 = 1_000_000;

const MAX_D_ITERATIONS: u32 = 255;

const MAX_NORMALIZED_RESERVE_WAD: u128 = 10u128.pow(34);

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
    let ann = U256::from_u128(env, amp).mul(&n);

    let mut d = sum.clone();
    for _ in 0..MAX_D_ITERATIONS {
        let mut d_p = d.clone();
        d_p = d_p.mul(&d).div(&xa.mul(&n));
        d_p = d_p.mul(&d).div(&xb.mul(&n));

        let d_prev = d.clone();
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
