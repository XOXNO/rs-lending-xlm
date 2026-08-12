//! Fair-value pricing for two-asset constant-product LP tokens, and the
//! integer square-root helper that computation relies on.

use crate::constants::WAD;
use crate::errors::OracleError;
use crate::math::fp_core::try_mul_div_half_up;
use crate::oracle::observation::{try_amount_to_wad, try_u256_to_i128};
use soroban_sdk::{Env, U256};

/// Computes `floor(sqrt(a * b))` as a `U256` using Newton's method, seeded
/// from `(isqrt(a) + 1) * (isqrt(b) + 1)`. Returns `a * b` directly when the
/// product is 0 or 1.
pub fn isqrt_of_product(env: &Env, a: u128, b: u128) -> U256 {
    let n = U256::from_u128(env, a).mul(&U256::from_u128(env, b));
    let one = U256::from_u32(env, 1);
    if n <= one {
        return n;
    }
    let two = U256::from_u32(env, 2);
    let seed = U256::from_u128(env, a.isqrt().saturating_add(1))
        .mul(&U256::from_u128(env, b.isqrt().saturating_add(1)));

    let mut x = seed;
    let mut y = x.add(&n.div(&x)).div(&two);
    while y < x {
        x = y.clone();
        y = x.add(&n.div(&x)).div(&two);
    }
    x
}

/// Reserve amount, token decimals, and WAD-scaled price for one asset leg of
/// an LP pool.
#[derive(Clone, Debug)]
pub struct LpLeg {
    pub reserve: i128,
    pub decimals: u32,
    pub price_wad: i128,
}

/// Total LP share supply and its decimals.
#[derive(Clone, Debug)]
pub struct LpSupply {
    pub total_shares: i128,
    pub decimals: u32,
}

/// Computes the fair-value price of one LP share, in WAD (1e18) scale.
///
/// Converts each leg's reserve into a WAD value (`reserve * price_wad /
/// 10^decimals`), combines the two leg values as `2 * sqrt(value_a *
/// value_b)`, and divides by the share supply converted to WAD. Returns
/// `OracleError::InvalidPrice` if any reserve, price, or share amount is not
/// positive, if a leg's value fails to compute, or if the result does not
/// fit in `i128`.
pub fn fair_lp_price_wad(
    env: &Env,
    a: &LpLeg,
    b: &LpLeg,
    supply: &LpSupply,
) -> Result<i128, OracleError> {
    if a.reserve <= 0
        || b.reserve <= 0
        || a.price_wad <= 0
        || b.price_wad <= 0
        || supply.total_shares <= 0
    {
        return Err(OracleError::InvalidPrice);
    }

    let value_a = reserve_value_wad(env, a)?;
    let value_b = reserve_value_wad(env, b)?;

    let total_value =
        isqrt_of_product(env, value_a as u128, value_b as u128).mul(&U256::from_u32(env, 2));

    let share_supply_wad = try_amount_to_wad(env, supply.total_shares, supply.decimals)?;
    if share_supply_wad <= 0 {
        return Err(OracleError::InvalidPrice);
    }
    let fair = total_value
        .mul(&U256::from_u128(env, WAD as u128))
        .div(&U256::from_u128(env, share_supply_wad as u128));

    try_u256_to_i128(&fair).ok_or(OracleError::InvalidPrice)
}

/// Converts one leg's reserve into a WAD-scaled value: `reserve * price_wad
/// / 10^decimals`, rounded half up. Returns `OracleError::InvalidPrice` if
/// `10^decimals` overflows `i128` or the multiply-divide fails.
fn reserve_value_wad(env: &Env, leg: &LpLeg) -> Result<i128, OracleError> {
    let denom = 10i128
        .checked_pow(leg.decimals)
        .ok_or(OracleError::InvalidPrice)?;
    try_mul_div_half_up(env, leg.reserve, leg.price_wad, denom).ok_or(OracleError::InvalidPrice)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leg(reserve: i128, price_wad: i128) -> LpLeg {
        LpLeg {
            reserve,
            decimals: 7,
            price_wad,
        }
    }

    fn supply(total_shares: i128) -> LpSupply {
        LpSupply {
            total_shares,
            decimals: 7,
        }
    }

    #[test]
    fn fair_price_matches_reference_snapshot() {
        let env = Env::default();
        let price = fair_lp_price_wad(
            &env,
            &leg(340_673_965, 110_000_000_000_000_000),
            &leg(58_315_575, 1_000_000_000_000_000_000),
            &supply(140_754_159),
        )
        .unwrap();
        assert!(
            price > 660_000_000_000_000_000 && price < 668_000_000_000_000_000,
            "fair price out of expected band: {price}"
        );
        assert!(price < 680_000_000_000_000_000);
    }

    #[test]
    fn absurd_reserve_errors_not_panics() {
        let env = Env::default();
        let err = fair_lp_price_wad(
            &env,
            &leg(i128::MAX, WAD),
            &leg(1_000_000_000, WAD),
            &supply(1_000_000_000),
        )
        .unwrap_err();
        assert_eq!(err, OracleError::InvalidPrice);
    }

    #[test]
    fn isqrt_matches_a_reference_root() {
        let env = Env::default();
        let check = |n: u128| {
            let got = isqrt_of_product(&env, n, 1);
            let exceeds = |v: u128| v.checked_mul(v).is_none_or(|square| square > n);
            let mut want = (n as f64).sqrt() as u128;
            while want > 0 && exceeds(want) {
                want -= 1;
            }
            while !exceeds(want + 1) {
                want += 1;
            }
            assert_eq!(
                got,
                U256::from_u128(&env, want),
                "isqrt({n}) should be {want}"
            );
        };
        for n in [
            0u128, 1, 2, 3, 4, 5, 8, 9, 15, 16, 17, 24, 25, 26, 99, 100, 101,
        ] {
            check(n);
        }
        for shift in [16u32, 32, 64, 96, 126] {
            let base = 1u128 << shift;
            check(base - 1);
            check(base);
            check(base + 1);
        }
        check(u128::MAX);
    }

    #[test]
    fn rejects_zero_supply() {
        let env = Env::default();
        let err = fair_lp_price_wad(&env, &leg(1, WAD), &leg(1, WAD), &supply(0)).unwrap_err();
        assert_eq!(err, OracleError::InvalidPrice);
    }

    #[test]
    fn balanced_pool_prices_at_reserve_value() {
        let env = Env::default();
        let price = fair_lp_price_wad(
            &env,
            &leg(1_000_000_000, WAD),
            &leg(1_000_000_000, WAD),
            &supply(1_000_000_000),
        )
        .unwrap();
        assert_eq!(price, 2_000_000_000_000_000_000);
    }
}
