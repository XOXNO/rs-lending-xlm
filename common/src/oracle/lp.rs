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

/// Integer square root of a `U256` via Newton's method. Converges in a handful
/// of iterations; each step is a few host arithmetic calls.
pub fn isqrt_u256(env: &Env, n: &U256) -> U256 {
    let one = U256::from_u32(env, 1);
    if n <= &one {
        return n.clone();
    }
    let two = U256::from_u32(env, 2);
    let mut x = n.clone();
    let mut y = n.add(&one).div(&two);
    while y < x {
        x = y.clone();
        y = x.add(&n.div(&x)).div(&two);
    }
    x
}

/// Fair USD price (WAD) of one whole LP share of a constant-product pool:
/// `2·sqrt(V_a·V_b) / S_whole`, where `V_i` is the USD value of reserve `i`.
///
/// Inputs are raw on-chain values: `reserve_i` in token base units, `price_i_wad`
/// USD per whole token (WAD), `total_shares` in LP base units.
///
/// # Errors
/// * [`OracleError::InvalidPrice`] - a non-positive reserve, price, or supply.
/// * [`OracleError::InvalidPrice`] - an intermediate exceeds its domain.
#[allow(clippy::too_many_arguments)]
pub fn fair_lp_price_wad(
    env: &Env,
    reserve_a: i128,
    reserve_a_decimals: u32,
    price_a_wad: i128,
    reserve_b: i128,
    reserve_b_decimals: u32,
    price_b_wad: i128,
    total_shares: i128,
    share_decimals: u32,
) -> Result<i128, OracleError> {
    if reserve_a <= 0
        || reserve_b <= 0
        || total_shares <= 0
        || price_a_wad <= 0
        || price_b_wad <= 0
    {
        return Err(OracleError::InvalidPrice);
    }

    // USD value of each reserve, WAD-scaled. Checked end-to-end: overflow or a
    // decimals > 18 maps to InvalidPrice, never a panic (a compromised/absurd
    // pool must not brick the price with an unrecoverable host error).
    let value_a = reserve_value_wad(env, reserve_a, reserve_a_decimals, price_a_wad)?;
    let value_b = reserve_value_wad(env, reserve_b, reserve_b_decimals, price_b_wad)?;

    // 2·sqrt(V_a·V_b): the product is up to ~1e52, past i128, so accumulate in U256.
    let product = U256::from_u128(env, value_a as u128).mul(&U256::from_u128(env, value_b as u128));
    let total_value = isqrt_u256(env, &product).mul(&U256::from_u32(env, 2)); // WAD USD

    // Per whole LP share (WAD) = total_value · WAD / share_supply_whole_wad.
    let share_supply_wad = amount_to_wad(env, total_shares, share_decimals)?;
    if share_supply_wad <= 0 {
        return Err(OracleError::InvalidPrice);
    }
    let fair = total_value
        .mul(&U256::from_u128(env, WAD as u128))
        .div(&U256::from_u128(env, share_supply_wad as u128));

    try_u256_to_i128(&fair).ok_or(OracleError::InvalidPrice)
}

/// `reserve · price_wad / 10^decimals` in WAD USD; `InvalidPrice` on overflow.
fn reserve_value_wad(
    env: &Env,
    reserve: i128,
    decimals: u32,
    price_wad: i128,
) -> Result<i128, OracleError> {
    let denom = 10i128
        .checked_pow(decimals)
        .ok_or(OracleError::InvalidPrice)?;
    try_mul_div_half_up(env, reserve, price_wad, denom).ok_or(OracleError::InvalidPrice)
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
#[allow(clippy::too_many_arguments)]
pub fn lp_sanity_band(
    env: &Env,
    reserve_a: i128,
    reserve_a_decimals: u32,
    reserve_b: i128,
    reserve_b_decimals: u32,
    total_shares: i128,
    share_decimals: u32,
    a_min_wad: i128,
    a_max_wad: i128,
    b_min_wad: i128,
    b_max_wad: i128,
) -> Result<(i128, i128), OracleError> {
    let lo = fair_lp_price_wad(
        env, reserve_a, reserve_a_decimals, a_min_wad, reserve_b, reserve_b_decimals, b_min_wad,
        total_shares, share_decimals,
    )?;
    let hi = fair_lp_price_wad(
        env, reserve_a, reserve_a_decimals, a_max_wad, reserve_b, reserve_b_decimals, b_max_wad,
        total_shares, share_decimals,
    )?;
    Ok((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real mainnet XLM/PYUSD constant-product pool snapshot: reserves
    // 340673965 / 58315575 (7 dp), supply 140754159 (7 dp). At XLM=$0.11,
    // PYUSD=$1.00 the fair share sits near $0.664 and BELOW the naive
    // (Va+Vb)/S = $0.680, because fair value penalises the imbalance.
    #[test]
    fn fair_price_matches_reference_snapshot() {
        let env = Env::default();
        let price = fair_lp_price_wad(
            &env,
            340_673_965,
            7,
            110_000_000_000_000_000, // $0.11 WAD
            58_315_575,
            7,
            1_000_000_000_000_000_000, // $1.00 WAD
            140_754_159,
            7,
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
            i128::MAX,
            7,
            WAD,
            1_000_000_000,
            7,
            WAD,
            1_000_000_000,
            7,
        )
        .unwrap_err();
        assert_eq!(err, OracleError::InvalidPrice);
    }

    #[test]
    fn rejects_zero_supply() {
        let env = Env::default();
        let err = fair_lp_price_wad(&env, 1, 7, WAD, 1, 7, WAD, 0, 7).unwrap_err();
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
            13_712_481_487,
            7,
            1_054_452_914_606,
            7,
            119_720_030_506,
            7,
            900_000_000_000_000_000,   // USDC min $0.90
            1_100_000_000_000_000_000, // USDC max $1.10
            45_000_000_000_000_000,    // XLM min $0.045
            500_000_000_000_000_000,   // XLM max $0.50
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
        let price =
            fair_lp_price_wad(&env, 1_000_000_000, 7, WAD, 1_000_000_000, 7, WAD, 1_000_000_000, 7)
                .unwrap();
        assert_eq!(price, 2_000_000_000_000_000_000);
    }
}
