//! Manipulation-resistant fair-value pricing for constant-product (`xy=k`) AMM
//! LP shares.
//!
//! The share value is priced from the pool invariant against external oracle
//! prices, so the reserve *ratio* — the only thing a flash-loan swap can move —
//! cancels out. Reserves enter solely through their product.

use crate::constants::{WAD, WAD_DECIMALS};
use crate::errors::OracleError;
use crate::math::fp_core::try_mul_div_half_up;
use crate::oracle::observation::try_u256_to_i128;
use soroban_sdk::{Env, U256};

/// Integer square root of `a·b`, where both factors fit `u128`.
///
/// Soroban has no sqrt host function and `U256` exposes none, so the root is
/// computed here. Only the product needs 256 bits — the seed does not, so the
/// factors' own `u128::isqrt` gives `(√a+1)·(√b+1)`, an over-estimate within a
/// hair of the true root. Newton then lands in one or two steps.
///
/// Seeding is the whole game: from `x0 = n` each step merely halves, so a real
/// LP product (~2^161) took ~86 U256 divisions and the worst case ~132.
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

/// One side of a constant-product pool: how much of the token the pool holds,
/// the token's own decimals, and its USD price. Grouping these keeps the three
/// values that must agree together, so a leg cannot be assembled half-swapped.
#[derive(Clone, Debug)]
pub struct LpLeg {
    pub reserve: i128,
    pub decimals: u32,
    pub price_wad: i128,
}

/// A leg whose price is only known within a band, used to derive the share's own
/// band from its underlyings'.
#[derive(Clone, Debug)]
pub struct LpLegBand {
    pub reserve: i128,
    pub decimals: u32,
    pub min_price_wad: i128,
    pub max_price_wad: i128,
}

impl LpLegBand {
    fn at(&self, price_wad: i128) -> LpLeg {
        LpLeg {
            reserve: self.reserve,
            decimals: self.decimals,
            price_wad,
        }
    }
}

/// The pool's share token: supply in base units, plus its decimals.
#[derive(Clone, Debug)]
pub struct LpSupply {
    pub total_shares: i128,
    pub decimals: u32,
}

/// Fair USD price (WAD) of one whole LP share of a constant-product pool:
/// `2·sqrt(V_a·V_b) / S_whole`, where `V_i` is the USD value of reserve `i`.
///
/// # Errors
/// * [`OracleError::InvalidPrice`] - a non-positive reserve, price, or supply,
///   or an intermediate that exceeds its domain.
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

    // USD value of each reserve, WAD-scaled. Checked end-to-end: overflow or a
    // decimals > 18 maps to InvalidPrice, never a panic (a compromised/absurd
    // pool must not brick the price with an unrecoverable host error).
    let value_a = reserve_value_wad(env, a)?;
    let value_b = reserve_value_wad(env, b)?;

    // 2·sqrt(V_a·V_b): the product is up to ~1e52, past i128, so it is rooted in U256.
    let total_value =
        isqrt_of_product(env, value_a as u128, value_b as u128).mul(&U256::from_u32(env, 2));

    // Per whole LP share (WAD) = total_value · WAD / share_supply_whole_wad.
    let share_supply_wad = amount_to_wad(env, supply.total_shares, supply.decimals)?;
    if share_supply_wad <= 0 {
        return Err(OracleError::InvalidPrice);
    }
    let fair = total_value
        .mul(&U256::from_u128(env, WAD as u128))
        .div(&U256::from_u128(env, share_supply_wad as u128));

    try_u256_to_i128(&fair).ok_or(OracleError::InvalidPrice)
}

/// `reserve · price_wad / 10^decimals` in WAD USD; `InvalidPrice` on overflow.
fn reserve_value_wad(env: &Env, leg: &LpLeg) -> Result<i128, OracleError> {
    let denom = 10i128
        .checked_pow(leg.decimals)
        .ok_or(OracleError::InvalidPrice)?;
    try_mul_div_half_up(env, leg.reserve, leg.price_wad, denom).ok_or(OracleError::InvalidPrice)
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

/// Derives an LP-share sanity band from the two underlyings' own sanity bands.
///
/// The fair value is monotonic in each underlying price, so the LP band is
/// `[fair(a_min, b_min), fair(a_max, b_max)]` at the current reserves/shares.
/// The pool factor `2·√k/S` is ~invariant to swaps and unchanged by
/// proportional deposits/withdraws, so the derived band stays valid as the pool
/// trades — no manual re-banding, and it only rejects prices the underlyings'
/// own bands already couldn't have produced (i.e. a compromised/degenerate pool).
///
/// # Errors
/// * [`OracleError::InvalidPrice`] - a non-positive reserve, band edge, or supply.
pub fn lp_sanity_band(
    env: &Env,
    a: &LpLegBand,
    b: &LpLegBand,
    supply: &LpSupply,
) -> Result<(i128, i128), OracleError> {
    let lo = fair_lp_price_wad(env, &a.at(a.min_price_wad), &b.at(b.min_price_wad), supply)?;
    let hi = fair_lp_price_wad(env, &a.at(a.max_price_wad), &b.at(b.max_price_wad), supply)?;
    Ok((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stellar assets are 7 decimals throughout these fixtures.
    fn leg(reserve: i128, price_wad: i128) -> LpLeg {
        LpLeg {
            reserve,
            decimals: 7,
            price_wad,
        }
    }

    fn band(reserve: i128, min_price_wad: i128, max_price_wad: i128) -> LpLegBand {
        LpLegBand {
            reserve,
            decimals: 7,
            min_price_wad,
            max_price_wad,
        }
    }

    fn supply(total_shares: i128) -> LpSupply {
        LpSupply {
            total_shares,
            decimals: 7,
        }
    }

    // Real mainnet XLM/PYUSD constant-product pool snapshot: reserves
    // 340673965 / 58315575 (7 dp), supply 140754159 (7 dp). At XLM=$0.11,
    // PYUSD=$1.00 the fair share sits near $0.664 and BELOW the naive
    // (Va+Vb)/S = $0.680, because fair value penalises the imbalance.
    #[test]
    fn fair_price_matches_reference_snapshot() {
        let env = Env::default();
        let price = fair_lp_price_wad(
            &env,
            &leg(340_673_965, 110_000_000_000_000_000), // XLM @ $0.11
            &leg(58_315_575, 1_000_000_000_000_000_000), // PYUSD @ $1.00
            &supply(140_754_159),
        )
        .unwrap();
        assert!(
            price > 660_000_000_000_000_000 && price < 668_000_000_000_000_000,
            "fair price out of expected band: {price}"
        );
        // Fair value is strictly below the manipulable spot valuation.
        assert!(price < 680_000_000_000_000_000);
    }

    // A pathologically large reserve/supply (e.g. a compromised plane) must map
    // to InvalidPrice, not an unrecoverable panic that would brick the market.
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

    // The seeded Newton must agree with a reference root across magnitudes, and
    // must land on the floor exactly at perfect squares and one either side.
    #[test]
    fn isqrt_matches_a_reference_root() {
        let env = Env::default();
        let check = |n: u128| {
            let got = isqrt_of_product(&env, n, 1);
            // checked_mul, not saturating: at n = u128::MAX a saturated square
            // still compares <= n, so the reference would climb forever.
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
        for n in [0u128, 1, 2, 3, 4, 5, 8, 9, 15, 16, 17, 24, 25, 26, 99, 100, 101] {
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
        let err =
            fair_lp_price_wad(&env, &leg(1, WAD), &leg(1, WAD), &supply(0)).unwrap_err();
        assert_eq!(err, OracleError::InvalidPrice);
    }

    // Auto-derived band for the real testnet XLM/USDC pool from the config
    // underlying bands (USDC $0.90-$1.10, XLM $0.045-$0.50). The current fair
    // price ($0.8319) must sit strictly inside, and the edges land near the
    // hand-computed $0.404 / $1.490.
    #[test]
    fn derived_band_brackets_current_fair_price() {
        let env = Env::default();
        let (lo, hi) = lp_sanity_band(
            &env,
            &band(13_712_481_487, 900_000_000_000_000_000, 1_100_000_000_000_000_000),
            &band(1_054_452_914_606, 45_000_000_000_000_000, 500_000_000_000_000_000),
            &supply(119_720_030_506),
        )
        .unwrap();
        let current = 831_864_415_970_608_854;
        assert!(lo < current && current < hi, "band [{lo}, {hi}] must bracket {current}");
        assert!(lo > 400_000_000_000_000_000 && lo < 408_000_000_000_000_000, "lo={lo}");
        assert!(hi > 1_485_000_000_000_000_000 && hi < 1_495_000_000_000_000_000, "hi={hi}");
    }

    #[test]
    fn balanced_pool_prices_at_reserve_value() {
        // Balanced pool, both sides $1: 1000 + 1000 units, supply 1000 → $2/share.
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
