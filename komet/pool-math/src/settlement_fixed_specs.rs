//! Repayment and withdrawal resolution at fixed borrow/supply indexes, for
//! 27-decimal assets (no decimal rescale) and amounts and positions of at
//! least 2^64 (every multiply-divide takes production's exact I256 path).
//! Specifications are independent I256 floor/ceil inequalities.

use common::{constants::RAY, math::fp::Ray, rates};
use soroban_sdk::{Env, I256};

pub const WIDE: i128 = 1 << 64;
/// Representative indexes: the initial index and an odd non-trivial one.
pub const INDEX_ONE: i128 = RAY;
pub const INDEX_MID: i128 = 3 * RAY / 2 + 7;

#[inline(never)]
fn repay(env: &Env, amount: i128, pos: i128, index: i128) -> (i128, i128) {
    let (burn, refund) = rates::resolve_repay(env, amount, Ray::from(pos), Ray::from(index), 27);
    (burn.raw(), refund)
}

#[inline(never)]
fn withdraw(env: &Env, amount: i128, pos: i128, index: i128) -> (i128, i128) {
    let (burn, paid) = rates::resolve_withdrawal(env, amount, Ray::from(pos), Ray::from(index), 27);
    (burn.raw(), paid)
}

fn w(env: &Env, v: i128) -> I256 {
    I256::from_i128(env, v)
}

/// q == floor(n / d) for n >= 0, d > 0.
fn is_floor(env: &Env, n: &I256, d: &I256, q: i128) -> bool {
    let q = w(env, q);
    q.mul(d) <= *n && *n < q.add(&w(env, 1)).mul(d)
}

/// q == ceil(n / d) for n >= 0, d > 0.
fn is_ceil(env: &Env, n: &I256, d: &I256, q: i128) -> bool {
    let q = w(env, q);
    q.sub(&w(env, 1)).mul(d) < *n && *n <= q.mul(d)
}

/// Domain: amount, pos >= 2^64 and the position's value fits i128.
fn in_domain(env: &Env, amount: i128, pos: i128, index: i128) -> bool {
    amount >= WIDE
        && pos >= WIDE
        && w(env, pos).mul(&w(env, index)) <= w(env, i128::MAX).mul(&w(env, RAY))
}

/// Full close when amount covers ceil(pos*I/RAY): burn everything and refund
/// the excess; otherwise burn floor(amount*RAY/I) (never more than pos) and
/// refund nothing.
pub fn repay_exact(env: &Env, amount: i128, pos: i128, index: i128) -> bool {
    if !in_domain(env, amount, pos, index) {
        return true;
    }
    let (burn, refund) = repay(env, amount, pos, index);
    let value = w(env, pos).mul(&w(env, index));
    let ray = w(env, RAY);
    let full = w(env, amount).mul(&ray) >= value; // amount >= value/RAY  <=> amount >= ceil(...)
    if full {
        let debt = amount - refund;
        burn == pos && refund >= 0 && is_ceil(env, &value, &ray, debt)
    } else {
        refund == 0 && burn <= pos && is_floor(env, &w(env, amount).mul(&ray), &w(env, index), burn)
    }
}

/// Withdrawal: full close when amount reaches the half-up value of the
/// position (pays its floor value, burns everything); otherwise burns
/// ceil(amount*RAY/I) and pays amount.
pub fn withdraw_exact(env: &Env, amount: i128, pos: i128, index: i128) -> bool {
    if !in_domain(env, amount, pos, index) {
        return true;
    }
    let (burn, paid) = withdraw(env, amount, pos, index);
    let value = w(env, pos).mul(&w(env, index));
    let ray = w(env, RAY);
    // half-up value h = floor((value + RAY/2) / RAY); amount >= h <=> amount*RAY + RAY/2 >= value + RAY/2 - ... use exact test:
    let half_up = value.add(&w(env, RAY / 2));
    let full = w(env, amount).add(&w(env, 1)).mul(&ray) > half_up;
    if full {
        burn == pos && is_floor(env, &value, &ray, paid)
    } else {
        paid == amount && is_ceil(env, &w(env, amount).mul(&ray), &w(env, index), burn)
    }
}

/// Deliberately false: a partial repayment's burn is claimed rounded up.
pub fn repay_wrong(env: &Env) -> bool {
    let (burn, _) = repay(env, WIDE, WIDE << 6, INDEX_MID);
    let n = w(env, WIDE).mul(&w(env, RAY));
    is_ceil(env, &n, &w(env, INDEX_MID), burn) && !is_floor(env, &n, &w(env, INDEX_MID), burn)
}

/// Deliberately false: a partial withdrawal's burn is claimed rounded down.
pub fn withdraw_wrong(env: &Env) -> bool {
    let (burn, _) = withdraw(env, WIDE, WIDE << 6, INDEX_MID);
    let n = w(env, WIDE).mul(&w(env, RAY));
    is_floor(env, &n, &w(env, INDEX_MID), burn) && !is_ceil(env, &n, &w(env, INDEX_MID), burn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settlement_fixed_witnesses() {
        let env = Env::default();
        for index in [INDEX_ONE, INDEX_MID] {
            for (a, p) in [
                (WIDE, WIDE),
                (WIDE, WIDE << 6),
                (WIDE << 6, WIDE),
                (RAY, RAY),
                (RAY * 3 / 2 + 11, RAY),
                (RAY + 1, RAY),
                (i128::MAX, WIDE),
                (WIDE, i128::MAX / 2),
            ] {
                assert!(repay_exact(&env, a, p, index), "repay {a} {p} {index}");
                assert!(
                    withdraw_exact(&env, a, p, index),
                    "withdraw {a} {p} {index}"
                );
            }
        }
        assert!(!repay_wrong(&env));
        assert!(!withdraw_wrong(&env));
    }
}
