//! Independent settlement specifications for production scaling helpers.
//!
//! Domains: nonnegative requests/positions, positive indexes, decimals <= 27,
//! and representable production i128 stages. Widened products stay in I256.
//! This covers the helper domain beyond the market listing limit of 18 decimals.
//! Excluded inputs are premises, not proofs of production error behavior.

use common::{
    constants::{RAY, RAY_DECIMALS},
    math::fp::Ray,
    rates,
};
use soroban_sdk::{Env, I256};

#[derive(Clone, Copy)]
enum Rounding {
    Floor,
    Ceil,
    HalfUp,
}

// Only nonnegative numerators and positive denominators reach this helper.
// An i128-by-i128 product plus any bias below i128::MAX fits signed I256.
fn round(env: &Env, numerator: &I256, denominator: i128, rounding: Rounding) -> I256 {
    let bias = match rounding {
        Rounding::Floor => 0,
        Rounding::Ceil => denominator - 1,
        Rounding::HalfUp => denominator / 2,
    };
    numerator
        .add(&I256::from_i128(env, bias))
        .div(&I256::from_i128(env, denominator))
}

fn domain(amount: i128, shares: i128, index: i128, decimals: u32) -> bool {
    amount >= 0 && shares >= 0 && index > 0 && decimals <= RAY_DECIMALS
}

fn factor(decimals: u32) -> i128 {
    10i128.pow(RAY_DECIMALS - decimals)
}

// Preserve both production rounding stages and their intermediate i128 bound.
// In particular, half-up(half-up(shares * index / RAY) / factor) can differ
// from one half-up division by RAY * factor.
fn value(env: &Env, shares: i128, index: i128, factor: i128, rounding: Rounding) -> Option<i128> {
    let product = I256::from_i128(env, shares).mul(&I256::from_i128(env, index));
    let original = round(env, &product, RAY, rounding).to_i128()?;
    round(env, &I256::from_i128(env, original), factor, rounding).to_i128()
}

// Check the actual asset-to-Ray i128 stage before multiplying by RAY again.
// Full settlement branches never call this, so a large request remains valid
// when the full-branch production code never rescales that request.
fn scaled(env: &Env, amount: i128, index: i128, factor: i128, rounding: Rounding) -> Option<i128> {
    let rescaled = I256::from_i128(env, amount).mul(&I256::from_i128(env, factor));
    rescaled.to_i128()?;
    round(
        env,
        &rescaled.mul(&I256::from_i128(env, RAY)),
        index,
        rounding,
    )
    .to_i128()
}

fn repay_expected(
    env: &Env,
    amount: i128,
    shares: i128,
    index: i128,
    decimals: u32,
) -> Option<(i128, i128)> {
    if !domain(amount, shares, index, decimals) {
        return None;
    }
    let factor = factor(decimals);
    let debt = value(env, shares, index, factor, Rounding::Ceil)?;
    if amount >= debt {
        Some((shares, amount - debt))
    } else {
        Some((scaled(env, amount, index, factor, Rounding::Floor)?, 0))
    }
}

fn withdrawal_expected(
    env: &Env,
    amount: i128,
    shares: i128,
    index: i128,
    decimals: u32,
) -> Option<(i128, i128)> {
    if !domain(amount, shares, index, decimals) {
        return None;
    }
    let factor = factor(decimals);
    let threshold = value(env, shares, index, factor, Rounding::HalfUp)?;
    let available = value(env, shares, index, factor, Rounding::Floor)?;
    if amount >= threshold {
        Some((shares, available))
    } else {
        Some((scaled(env, amount, index, factor, Rounding::Ceil)?, amount))
    }
}

fn net_settle_expected(
    env: &Env,
    amount: i128,
    supply: i128,
    debt: i128,
    supply_index: i128,
    borrow_index: i128,
    decimals: u32,
) -> Option<(i128, i128, i128)> {
    if !domain(amount, supply, supply_index, decimals)
        || !domain(amount, debt, borrow_index, decimals)
    {
        return None;
    }
    let factor = factor(decimals);
    let available = value(env, supply, supply_index, factor, Rounding::Floor)?;
    let owed = value(env, debt, borrow_index, factor, Rounding::Ceil)?;
    let settled = amount.min(available).min(owed);
    if settled == 0 {
        return Some((0, 0, 0));
    }
    let supply_burn = if settled == available {
        supply
    } else {
        scaled(env, settled, supply_index, factor, Rounding::Ceil)?.min(supply)
    };
    let debt_burn = if settled == owed {
        debt
    } else {
        scaled(env, settled, borrow_index, factor, Rounding::Floor)?.min(debt)
    };
    Some((supply_burn, debt_burn, settled))
}

fn pair_matches(expected: (i128, i128), actual: (i128, i128), shares: i128, amount: i128) -> bool {
    actual == expected && actual.0 >= 0 && actual.0 <= shares && actual.1 >= 0 && actual.1 <= amount
}

fn net_matches(
    expected: (i128, i128, i128),
    actual: (i128, i128, i128),
    supply: i128,
    debt: i128,
    amount: i128,
) -> bool {
    actual == expected
        && actual.0 >= 0
        && actual.0 <= supply
        && actual.1 >= 0
        && actual.1 <= debt
        && actual.2 >= 0
        && actual.2 <= amount
}

#[inline(never)]
fn repay(env: &Env, amount: i128, shares: i128, index: i128, decimals: u32) -> (i128, i128) {
    let (burn, refund) =
        rates::resolve_repay(env, amount, Ray::from(shares), Ray::from(index), decimals);
    (burn.raw(), refund)
}

#[inline(never)]
fn withdrawal(env: &Env, amount: i128, shares: i128, index: i128, decimals: u32) -> (i128, i128) {
    let (burn, paid) =
        rates::resolve_withdrawal(env, amount, Ray::from(shares), Ray::from(index), decimals);
    (burn.raw(), paid)
}

#[inline(never)]
fn net_settle(
    env: &Env,
    amount: i128,
    supply: i128,
    debt: i128,
    supply_index: i128,
    borrow_index: i128,
    decimals: u32,
) -> (i128, i128, i128) {
    let (supply_burn, debt_burn, settled) = rates::resolve_net_settle(
        env,
        amount,
        Ray::from(supply),
        Ray::from(debt),
        Ray::from(supply_index),
        Ray::from(borrow_index),
        decimals,
    );
    (supply_burn.raw(), debt_burn.raw(), settled)
}

/// Exact burn/refund, including full repayment and excess refunds.
/// A partial repayment floors its scaled burn; every admitted burn <= shares.
pub fn repay_exact(env: &Env, amount: i128, shares: i128, index: i128, decimals: u32) -> bool {
    let Some(expected) = repay_expected(env, amount, shares, index, decimals) else {
        return true;
    };
    pair_matches(
        expected,
        repay(env, amount, shares, index, decimals),
        shares,
        amount,
    )
}

/// Exact burn/payout with sequential half-up full-withdrawal threshold and
/// floor full payout. Partial withdrawals ceil their burn; every burn <= shares.
pub fn withdrawal_exact(env: &Env, amount: i128, shares: i128, index: i128, decimals: u32) -> bool {
    let Some(expected) = withdrawal_expected(env, amount, shares, index, decimals) else {
        return true;
    };
    pair_matches(
        expected,
        withdrawal(env, amount, shares, index, decimals),
        shares,
        amount,
    )
}

/// Exact overlap, zero/full/partial side decisions, and both position burn bounds.
pub fn net_settle_exact(
    env: &Env,
    amount: i128,
    supply: i128,
    debt: i128,
    supply_index: i128,
    borrow_index: i128,
    decimals: u32,
) -> bool {
    let Some(expected) = net_settle_expected(
        env,
        amount,
        supply,
        debt,
        supply_index,
        borrow_index,
        decimals,
    ) else {
        return true;
    };
    net_matches(
        expected,
        net_settle(
            env,
            amount,
            supply,
            debt,
            supply_index,
            borrow_index,
            decimals,
        ),
        supply,
        debt,
        amount,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repay_branches_and_refunds_are_nonvacuous() {
        let env = Env::default();
        let f = factor(18);
        for (amount, shares, index, expected) in [
            (0, 0, RAY, (0, 0)),
            (7, 0, RAY, (0, 7)),
            (0, f, RAY, (0, 0)),
            (1, f + 1, RAY, (f, 0)),
            (2, f + 1, RAY, (f + 1, 0)),
            (3, f + 1, RAY, (f + 1, 1)),
            (1, f, 3 * RAY, (f / 3, 0)),
            (1, 1, RAY + 1, (1, 0)),
            (i128::MAX, 1, RAY, (1, i128::MAX - 1)),
        ] {
            assert_eq!(
                repay_expected(&env, amount, shares, index, 18),
                Some(expected)
            );
            assert_eq!(repay(&env, amount, shares, index, 18), expected);
            assert!(repay_exact(&env, amount, shares, index, 18));
            assert!(!pair_matches(
                expected,
                (expected.0 + 1, expected.1),
                shares,
                amount
            ));
            assert!(!pair_matches(
                expected,
                (expected.0, expected.1 + 1),
                shares,
                amount
            ));
        }
    }

    #[test]
    fn withdrawal_preserves_sequential_half_up_and_floor_payout() {
        let env = Env::default();
        let f = factor(18);
        for (amount, shares, index, expected) in [
            (0, 0, RAY, (0, 0)),
            (0, f, RAY, (0, 0)),
            (0, f / 2 - 1, RAY, (f / 2 - 1, 0)),
            (0, f / 2, RAY - 1, (0, 0)),
            (1, f / 2, RAY - 1, (f / 2, 0)),
            (1, 3 * f / 2, RAY - 1, (f + 1, 1)),
            (1, f + 1, RAY, (f + 1, 1)),
            (1, 2 * f, RAY, (f, 1)),
            (1, f, 3 * RAY, (f / 3 + 1, 1)),
            (1, 1, 2 * f * RAY - 1, (1, 1)),
            (i128::MAX, 1, RAY, (1, 0)),
        ] {
            assert_eq!(
                withdrawal_expected(&env, amount, shares, index, 18),
                Some(expected)
            );
            assert_eq!(withdrawal(&env, amount, shares, index, 18), expected);
            assert!(withdrawal_exact(&env, amount, shares, index, 18));
            assert!(!pair_matches(
                expected,
                (expected.0 + 1, expected.1),
                shares,
                amount
            ));
            assert!(!pair_matches(
                expected,
                (expected.0, expected.1 + 1),
                shares,
                amount
            ));
        }
        // Collapsing the two half-up stages changes the amount=0 branch.
        let product = I256::from_i128(&env, f / 2).mul(&I256::from_i128(&env, RAY - 1));
        let divisor = I256::from_i128(&env, RAY).mul(&I256::from_i128(&env, f));
        assert_eq!(
            product
                .add(&divisor.div(&I256::from_i128(&env, 2)))
                .div(&divisor)
                .to_i128(),
            Some(0)
        );
        assert_eq!(value(&env, f / 2, RAY - 1, f, Rounding::HalfUp), Some(1));
        assert!(!pair_matches((0, 0), (f / 2, 0), f / 2, 0));
        assert!(!pair_matches((f + 1, 1), (3 * f / 2, 1), 3 * f / 2, 1));
        // Substituting floor for the partial supply burn must be detected.
        assert!(!pair_matches((f / 3 + 1, 1), (f / 3, 1), f, 1));
    }

    #[test]
    fn net_settle_covers_each_full_partial_and_zero_side() {
        let env = Env::default();
        let f = factor(18);
        for (amount, supply, debt, si, bi, expected) in [
            (0, f, f, RAY, RAY, (0, 0, 0)),
            (1, 0, f, RAY, RAY, (0, 0, 0)),
            (1, f, 0, RAY, RAY, (0, 0, 0)),
            (1, f / 2, f, RAY, RAY, (0, 0, 0)),
            (1, 3 * f, 3 * f, RAY, RAY, (f, f, 1)),
            (5, 2 * f, 3 * f, RAY, RAY, (2 * f, 2 * f, 2)),
            (5, 3 * f, 2 * f, RAY, RAY, (2 * f, 2 * f, 2)),
            (5, 2 * f, 2 * f, RAY, RAY, (2 * f, 2 * f, 2)),
            (1, f + 1, 2 * f + 1, RAY, RAY, (f + 1, f, 1)),
            (2, 3 * f, f + 1, RAY, RAY, (2 * f, f + 1, 2)),
            (1, 2 * f, 2 * f, 3 * RAY, 3 * RAY, (f / 3 + 1, f / 3, 1)),
            (1, f, 1, RAY, 2 * f * RAY, (f, 0, 1)),
        ] {
            assert_eq!(
                net_settle_expected(&env, amount, supply, debt, si, bi, 18),
                Some(expected)
            );
            assert_eq!(net_settle(&env, amount, supply, debt, si, bi, 18), expected);
            assert!(net_settle_exact(&env, amount, supply, debt, si, bi, 18));
            assert!(!net_matches(
                expected,
                (expected.0 + 1, expected.1, expected.2),
                supply,
                debt,
                amount
            ));
            assert!(!net_matches(
                expected,
                (expected.0, expected.1 + 1, expected.2),
                supply,
                debt,
                amount
            ));
            assert!(!net_matches(
                expected,
                (expected.0, expected.1, expected.2 + 1),
                supply,
                debt,
                amount
            ));
        }
        // Swapped rounding directions and a missed full-side dust burn fail.
        assert!(!net_matches(
            (f / 3 + 1, f / 3, 1),
            (f / 3, f / 3 + 1, 1),
            2 * f,
            2 * f,
            1
        ));
        assert!(!net_matches((f + 1, f, 1), (f, f, 1), f + 1, 2 * f + 1, 1));
    }

    #[test]
    fn supported_decimal_and_representability_boundaries() {
        let env = Env::default();
        for decimals in [0, 7, 18, 26, 27] {
            let f = factor(decimals);
            assert_eq!(repay_expected(&env, 1, 2 * f, RAY, decimals), Some((f, 0)));
            assert_eq!(
                withdrawal_expected(&env, 1, 2 * f, RAY, decimals),
                Some((f, 1))
            );
            assert_eq!(
                net_settle_expected(&env, 1, 2 * f, 2 * f, RAY, RAY, decimals),
                Some((f, f, 1))
            );
            assert!(repay_exact(&env, 1, 2 * f, RAY, decimals));
            assert!(withdrawal_exact(&env, 1, 2 * f, RAY, decimals));
            assert!(net_settle_exact(&env, 1, 2 * f, 2 * f, RAY, RAY, decimals));
        }
        // At 27 decimals the second stage is identity, including half-up.
        assert_eq!(
            withdrawal_expected(&env, 0, 1, RAY / 2 - 1, 27),
            Some((1, 0))
        );
        assert_eq!(withdrawal_expected(&env, 0, 1, RAY / 2, 27), Some((0, 0)));
        assert_eq!(repay_expected(&env, 1, 1, RAY + 1, 27), Some((0, 0)));
        assert_eq!(
            net_settle_expected(&env, 1, 1, 1, 3 * RAY, 3 * RAY, 27),
            Some((1, 0, 1))
        );
        assert!(withdrawal_exact(&env, 0, 1, RAY / 2 - 1, 27));
        assert!(withdrawal_exact(&env, 0, 1, RAY / 2, 27));
        assert!(repay_exact(&env, 1, 1, RAY + 1, 27));
        assert!(net_settle_exact(&env, 1, 1, 1, 3 * RAY, 3 * RAY, 27));
        // Rounded valuation can reach i128::MAX without excluding the input.
        let f = factor(18);
        let floor_max = i128::MAX / f;
        assert_eq!(
            repay_expected(&env, i128::MAX, i128::MAX, RAY, 18),
            Some((i128::MAX, i128::MAX - floor_max - 1))
        );
        assert_eq!(
            withdrawal_expected(&env, i128::MAX, i128::MAX, RAY, 18),
            Some((i128::MAX, floor_max))
        );
        assert_eq!(
            net_settle_expected(&env, i128::MAX, i128::MAX, i128::MAX, RAY, RAY, 18),
            Some((i128::MAX, floor_max * f, floor_max))
        );
        assert!(repay_exact(&env, i128::MAX, i128::MAX, RAY, 18));
        assert!(withdrawal_exact(&env, i128::MAX, i128::MAX, RAY, 18));
        assert!(net_settle_exact(
            &env,
            i128::MAX,
            i128::MAX,
            i128::MAX,
            RAY,
            RAY,
            18
        ));
        // Full branches must admit requests whose unused rescale would overflow.
        assert_eq!(
            repay_expected(&env, i128::MAX, 1, RAY, 0),
            Some((1, i128::MAX - 1))
        );
        assert_eq!(
            withdrawal_expected(&env, i128::MAX, 1, RAY, 0),
            Some((1, 0))
        );
        assert!(repay_exact(&env, i128::MAX, 1, RAY, 0));
        assert!(withdrawal_exact(&env, i128::MAX, 1, RAY, 0));
        assert!(repay_expected(&env, 1, i128::MAX, 2 * RAY, 18).is_none());
        assert!(withdrawal_expected(&env, 1, i128::MAX, 2 * RAY, 18).is_none());
        assert!(net_settle_expected(&env, 1, i128::MAX, 1, 2 * RAY, RAY, 18).is_none());
        assert!(repay_expected(&env, 1, 1, 0, 18).is_none());
        assert!(repay_expected(&env, 1, 1, RAY, 28).is_none());
        assert!(withdrawal_expected(&env, -1, 1, RAY, 18).is_none());
        assert!(net_settle_expected(&env, 1, -1, 1, RAY, RAY, 18).is_none());
    }
}
