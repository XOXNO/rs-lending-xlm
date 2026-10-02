#![no_std]

#[cfg(any(
    feature = "index-exact",
    feature = "supply-index",
    feature = "reward-conservation",
    feature = "interest-split",
    feature = "fee-shares",
    feature = "flash-fee"
))]
pub mod index_specs;
#[cfg(any(feature = "repay", feature = "withdrawal", feature = "net-settle"))]
pub mod settlement_specs;
#[cfg(any(
    feature = "signed-floor",
    feature = "signed-ceil",
    feature = "signed-saturation",
    feature = "scaled-supply",
    feature = "scaled-borrow"
))]
pub mod signed;

#[cfg(any(feature = "borrow-index", feature = "borrow-double"))]
use common::constants::MAX_BORROW_INDEX_RAY;
#[cfg(any(feature = "half-up", feature = "half-two", feature = "controls"))]
use common::math::fp_core;
#[cfg(any(
    feature = "utilization",
    feature = "borrow-index",
    feature = "borrow-double",
    feature = "controls"
))]
use common::{constants::RAY, math::fp::Ray, rates};
#[cfg(feature = "half-up")]
use soroban_sdk::I256;
use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct PoolMathProof;

#[inline(never)]
#[cfg(feature = "half-two")]
fn production_half(env: &Env, amount: i128) -> i128 {
    fp_core::div_by_int_half_up(env, amount, 2)
}

#[cfg(all(test, feature = "half-two"))]
#[test]
fn half_two_signed_extrema_and_ties() {
    let env = Env::default();
    for amount in [i128::MIN, i128::MIN + 1, -3, -2, -1, 0, 1, 2, 3, i128::MAX] {
        assert!(PoolMathProof::test_half_two(env.clone(), amount));
    }
    assert_eq!(production_half(&env, -1), -1);
    assert_eq!(production_half(&env, 1), 1);
    assert!(!PoolMathProof::test_half_two_wrong(env));
}

// Retain a production call boundary so the assertion is not constant-folded.
#[inline(never)]
#[cfg(any(feature = "utilization", feature = "controls"))]
fn production_utilization(env: &Env, borrowed: i128, supplied: i128) -> i128 {
    rates::utilization(env, Ray::from(borrowed), Ray::from(supplied)).raw()
}

#[cfg(all(test, feature = "borrow-double"))]
mod tests {
    use super::*;

    #[test]
    fn borrow_double_has_witnesses_on_both_cap_branches() {
        let env = Env::default();
        for old in [
            RAY,
            MAX_BORROW_INDEX_RAY / 2 - 1,
            MAX_BORROW_INDEX_RAY / 2,
            MAX_BORROW_INDEX_RAY / 2 + 1,
            MAX_BORROW_INDEX_RAY,
        ] {
            assert!((RAY..=MAX_BORROW_INDEX_RAY).contains(&old));
            assert!(PoolMathProof::test_borrow_double(env.clone(), old));
        }
        assert!(!PoolMathProof::test_borrow_double_wrong(env));
    }
}

#[inline(never)]
#[cfg(any(feature = "borrow-index", feature = "borrow-double"))]
fn production_borrow_index(env: &Env, old: i128, factor: i128) -> i128 {
    rates::update_borrow_index(env, Ray::from(old), Ray::from(factor)).raw()
}

#[inline(never)]
#[cfg(feature = "controls")]
fn production_ceil(env: &Env, x: i128, y: i128, divisor: i128) -> i128 {
    fp_core::mul_div_ceil(env, x, y, divisor)
}

#[contractimpl]
impl PoolMathProof {
    /// Every signed i128: division by two rounds ties away from zero.
    /// This fixed-divisor theorem supplements the general multiply-divide claim.
    #[cfg(feature = "half-two")]
    pub fn test_half_two(env: Env, amount: i128) -> bool {
        production_half(&env, amount) == amount / 2 + amount % 2
    }

    /// Deliberately false tie-to-zero result for one half.
    #[cfg(feature = "half-two")]
    pub fn test_half_two_wrong(env: Env) -> bool {
        production_half(&env, 1) == 0
    }

    /// All valid borrow indexes, at a fixed factor of two, including the cap.
    #[cfg(feature = "borrow-double")]
    pub fn test_borrow_double(env: Env, old: i128) -> bool {
        if !(RAY..=MAX_BORROW_INDEX_RAY).contains(&old) {
            return true;
        }
        production_borrow_index(&env, old, 2 * RAY) == (2 * old).min(MAX_BORROW_INDEX_RAY)
    }

    /// Deliberately false uncapped result just above the saturation boundary.
    #[cfg(feature = "borrow-double")]
    pub fn test_borrow_double_wrong(env: Env) -> bool {
        let old = MAX_BORROW_INDEX_RAY / 2 + 1;
        production_borrow_index(&env, old, 2 * RAY) == 2 * old
    }

    #[cfg(feature = "signed-floor")]
    pub fn test_floor_exact(env: Env, x: i128, y: i128, divisor: i128) -> bool {
        signed::floor_exact(&env, x, y, divisor)
    }

    #[cfg(feature = "signed-ceil")]
    pub fn test_ceil_exact(env: Env, x: i128, y: i128, divisor: i128) -> bool {
        signed::ceil_exact(&env, x, y, divisor)
    }

    #[cfg(feature = "signed-saturation")]
    pub fn test_saturating_floor(env: Env, x: i128, y: i128, divisor: i128) -> bool {
        signed::floor_saturating_exact(&env, x, y, divisor)
    }

    #[cfg(feature = "scaled-supply")]
    pub fn test_scaled_supply(env: Env, amount: i128, decimals: u32, index: i128) -> bool {
        signed::scaled_supply_exact(&env, amount, decimals, index)
    }

    #[cfg(feature = "scaled-borrow")]
    pub fn test_scaled_borrow(env: Env, amount: i128, decimals: u32, index: i128) -> bool {
        signed::scaled_borrow_exact(&env, amount, decimals, index)
    }

    #[cfg(feature = "index-exact")]
    pub fn test_borrow_index_exact(env: Env, old: i128, factor: i128) -> bool {
        index_specs::borrow_index_exact(&env, old, factor)
    }

    #[cfg(feature = "supply-index")]
    pub fn test_supply_index_exact(env: Env, supplied: i128, old: i128, rewards: i128) -> bool {
        index_specs::supply_index_exact(&env, supplied, old, rewards)
    }

    #[cfg(feature = "reward-conservation")]
    pub fn test_supply_rewards(env: Env, supplied: i128, old: i128, rewards: i128) -> bool {
        index_specs::supply_reward_conservation(&env, supplied, old, rewards)
    }

    #[cfg(feature = "interest-split")]
    pub fn test_interest_split(
        env: Env,
        asset: soroban_sdk::Address,
        borrowed: i128,
        old: i128,
        new: i128,
        reserve_bps: u32,
    ) -> bool {
        index_specs::interest_split_exact(&env, asset, borrowed, old, new, reserve_bps)
    }

    #[cfg(feature = "fee-shares")]
    pub fn test_fee_shares(env: Env, fee: i128, index: i128, supplied: i128) -> bool {
        index_specs::fee_shares_exact(&env, fee, index, supplied)
    }

    #[cfg(feature = "flash-fee")]
    pub fn test_flash_fee(env: Env, amount: i128, fee_bps: u32) -> bool {
        index_specs::flash_fee_exact(&env, amount, fee_bps)
    }

    #[cfg(feature = "repay")]
    pub fn test_repay(env: Env, amount: i128, shares: i128, index: i128, decimals: u32) -> bool {
        settlement_specs::repay_exact(&env, amount, shares, index, decimals)
    }

    #[cfg(feature = "withdrawal")]
    pub fn test_withdrawal(
        env: Env,
        amount: i128,
        shares: i128,
        index: i128,
        decimals: u32,
    ) -> bool {
        settlement_specs::withdrawal_exact(&env, amount, shares, index, decimals)
    }

    #[cfg(feature = "net-settle")]
    pub fn test_net_settle(
        env: Env,
        amount: i128,
        supply: i128,
        debt: i128,
        supply_index: i128,
        borrow_index: i128,
        decimals: u32,
    ) -> bool {
        settlement_specs::net_settle_exact(
            &env,
            amount,
            supply,
            debt,
            supply_index,
            borrow_index,
            decimals,
        )
    }

    /// Exact half-up result and rejection domain, specified without division.
    #[cfg(feature = "half-up")]
    pub fn test_half_up_exact(env: Env, x: i128, y: i128, divisor: i128) -> bool {
        let result = fp_core::try_mul_div_half_up(&env, x, y, divisor);
        if x < 0 || y < 0 || divisor <= 0 {
            return result.is_none();
        }

        // i128 operands keep these independent integer inequalities inside I256.
        let denominator = I256::from_i128(&env, divisor);
        let numerator = I256::from_i128(&env, x)
            .mul(&I256::from_i128(&env, y))
            .add(&I256::from_i128(&env, divisor / 2));
        let one = I256::from_i128(&env, 1);
        match result {
            Some(value) => {
                let quotient = I256::from_i128(&env, value);
                value >= 0
                    && quotient.mul(&denominator) <= numerator
                    && numerator < quotient.add(&one).mul(&denominator)
            }
            None => {
                let first_unrepresentable = I256::from_i128(&env, i128::MAX).add(&one);
                numerator >= first_unrepresentable.mul(&denominator)
            }
        }
    }

    /// For all 0 <= borrowed <= supplied, supplied > 0: utilization is in [0, RAY].
    #[cfg(feature = "utilization")]
    pub fn test_utilization_bounds(env: Env, borrowed: i128, supplied: i128) -> bool {
        if borrowed < 0 || supplied <= 0 || borrowed > supplied {
            return true;
        }
        let result = production_utilization(&env, borrowed, supplied);
        result >= 0 && result <= RAY
    }

    /// A zero denominator returns zero, for every signed i128 borrowed amount.
    #[cfg(feature = "controls")]
    pub fn test_util_zero_supply(env: Env, borrowed: i128) -> bool {
        production_utilization(&env, borrowed, 0) == 0
    }

    /// Index monotonicity and cap for RAY <= old <= cap and RAY <= factor <= 8 RAY.
    #[cfg(feature = "borrow-index")]
    pub fn test_borrow_index_bounds(env: Env, old: i128, factor: i128) -> bool {
        if old < RAY || old > MAX_BORROW_INDEX_RAY || factor < RAY || factor > 8 * RAY {
            return true;
        }
        let result = production_borrow_index(&env, old, factor);
        result >= old && result <= MAX_BORROW_INDEX_RAY
    }

    /// Deliberately false strict upper bound: full utilization equals RAY.
    #[cfg(feature = "controls")]
    pub fn test_utilization_wrong(env: Env, borrowed: i128, supplied: i128) -> bool {
        if borrowed < 0 || supplied <= 0 || borrowed > supplied {
            return true;
        }
        production_utilization(&env, borrowed, supplied) < RAY
    }

    /// Concrete nonvacuity witness inside the utilization theorem's domain.
    #[cfg(feature = "controls")]
    pub fn test_full_utilization(env: Env) -> bool {
        production_utilization(&env, RAY, RAY) == RAY
    }

    /// The same witness violates the deliberately false strict upper bound.
    #[cfg(feature = "controls")]
    pub fn test_full_utilization_wrong(env: Env) -> bool {
        production_utilization(&env, RAY, RAY) < RAY
    }

    /// Concrete widened path: (RAY + 1)^2 / RAY has quotient RAY + 2, remainder 1.
    #[cfg(feature = "controls")]
    pub fn test_widened_ceil(env: Env) -> bool {
        production_ceil(&env, RAY + 1, RAY + 1, RAY) == RAY + 3
    }
}
