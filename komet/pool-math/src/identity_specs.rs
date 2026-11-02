//! Identity, cap and no-op claims for production index and fixed-point
//! helpers (Certora counterparts: `borrow_index_identity_is_noop`,
//! `borrow_index_cap_is_sticky`, `supply_index_zero_inputs_are_noop`,
//! `compound_interest_identity_at_zero_delta`,
//! `scaled_price_rejects_negative_operands`, `rescale_upscale_lossless`).
//! Each positive claim has a deliberately false control.

use common::{
    constants::{MAX_BORROW_INDEX_RAY, RAY},
    math::fp::{Ray, Wad},
    rates,
};
use soroban_sdk::{Env, I256};

#[inline(never)]
fn borrow_index(env: &Env, old: i128, factor: i128) -> i128 {
    rates::update_borrow_index(env, Ray::from(old), Ray::from(factor)).raw()
}

#[inline(never)]
fn supply_index(env: &Env, supplied: i128, old: i128, rewards: i128) -> i128 {
    rates::update_supply_index(env, Ray::from(supplied), Ray::from(old), Ray::from(rewards)).raw()
}

#[inline(never)]
fn compound(env: &Env, rate: i128, delta_ms: u64) -> i128 {
    rates::compound_interest(env, Ray::from(rate), delta_ms).raw()
}

#[inline(never)]
fn wad_try_mul(env: &Env, a: i128, b: i128) -> Option<i128> {
    Wad::from(a).try_mul(env, Wad::from(b)).map(Wad::raw)
}

#[inline(never)]
fn ray_from_asset(env: &Env, amount: i128, decimals: u32) -> i128 {
    Ray::from_asset(env, amount, decimals).raw()
}

/// Every valid index: growth by exactly one RAY leaves it unchanged.
pub fn borrow_index_identity(env: &Env, old: i128) -> bool {
    if !(RAY..=MAX_BORROW_INDEX_RAY).contains(&old) {
        return true;
    }
    borrow_index(env, old, RAY) == old
}

/// Deliberately false: identity growth does not round RAY+1 down to RAY.
pub fn borrow_index_identity_wrong(env: &Env) -> bool {
    borrow_index(env, RAY + 1, RAY) == RAY
}

/// At the cap, every factor in (RAY, 100*RAY] keeps the index at the cap.
/// (100*RAY keeps the pre-cap product representable: 10^38 < i128::MAX.)
pub fn borrow_index_cap_sticky(env: &Env, factor: i128) -> bool {
    if factor <= RAY || factor > 100 * RAY {
        return true;
    }
    borrow_index(env, MAX_BORROW_INDEX_RAY, factor) == MAX_BORROW_INDEX_RAY
}

/// Deliberately false: the cap is not exceeded by doubling.
pub fn borrow_index_cap_sticky_wrong(env: &Env) -> bool {
    borrow_index(env, MAX_BORROW_INDEX_RAY, 2 * RAY) == 2 * MAX_BORROW_INDEX_RAY
}

/// Every i128 triple: zero supply or zero rewards returns the old index.
pub fn supply_index_zero_inputs(env: &Env, supplied: i128, old: i128, rewards: i128) -> bool {
    if supplied != 0 && rewards != 0 {
        return true;
    }
    supply_index(env, supplied, old, rewards) == old
}

/// Deliberately false: zero supply does not grow the index.
pub fn supply_index_zero_inputs_wrong(env: &Env) -> bool {
    supply_index(env, 0, RAY, RAY) == RAY + 1
}

/// Every i128 rate: a zero elapsed time gives exactly the unit factor.
pub fn compound_zero_delta(env: &Env, rate: i128) -> bool {
    compound(env, rate, 0) == RAY
}

/// Deliberately false: a zero elapsed time does not add the rate.
pub fn compound_zero_delta_wrong(env: &Env) -> bool {
    compound(env, 1, 0) == RAY + 1
}

/// Every i128 pair with a negative operand: Wad::try_mul returns None.
pub fn wad_try_mul_negative(env: &Env, a: i128, b: i128) -> bool {
    if a >= 0 && b >= 0 {
        return true;
    }
    wad_try_mul(env, a, b).is_none()
}

/// Deliberately false: a negative operand is not multiplied.
pub fn wad_try_mul_negative_wrong(env: &Env) -> bool {
    wad_try_mul(env, -1, 0) == Some(0)
}

/// 7-decimal asset to Ray: exact multiplication by 10^20 for every signed
/// amount whose product fits i128 (checked independently in I256).
pub fn from_asset_7_exact(env: &Env, amount: i128) -> bool {
    let factor = I256::from_i128(env, 100_000_000_000_000_000_000);
    let product = I256::from_i128(env, amount).mul(&factor);
    if product > I256::from_i128(env, i128::MAX) || product < I256::from_i128(env, i128::MIN) {
        return true;
    }
    I256::from_i128(env, ray_from_asset(env, amount, 7)) == product
}

/// Deliberately false: one 7-decimal unit is 10^20 Ray, not 10^19.
pub fn from_asset_7_wrong(env: &Env) -> bool {
    ray_from_asset(env, 1, 7) == 10_000_000_000_000_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_witnesses_and_controls() {
        let env = Env::default();
        for old in [
            RAY,
            RAY + 1,
            3 * RAY / 2,
            MAX_BORROW_INDEX_RAY - 1,
            MAX_BORROW_INDEX_RAY,
        ] {
            assert!(borrow_index_identity(&env, old));
        }
        for factor in [RAY + 1, 2 * RAY, 100 * RAY] {
            assert!(borrow_index_cap_sticky(&env, factor));
        }
        for (s, o, r) in [
            (0, RAY, RAY),
            (RAY, RAY, 0),
            (0, -5, 7),
            (0, 0, 0),
            (i128::MAX, i128::MIN, 0),
        ] {
            assert!(supply_index_zero_inputs(&env, s, o, r));
        }
        for rate in [i128::MIN, -1, 0, 1, RAY, i128::MAX] {
            assert!(compound_zero_delta(&env, rate));
        }
        for (a, b) in [(-1, 0), (0, -1), (i128::MIN, i128::MAX), (-RAY, -RAY)] {
            assert!(wad_try_mul_negative(&env, a, b));
        }
        let bound = i128::MAX / 100_000_000_000_000_000_000;
        for amount in [
            0,
            1,
            -1,
            bound,
            -bound,
            i128::MIN / 100_000_000_000_000_000_000,
        ] {
            assert!(from_asset_7_exact(&env, amount));
        }
        assert!(!borrow_index_identity_wrong(&env));
        assert!(!borrow_index_cap_sticky_wrong(&env));
        assert!(!supply_index_zero_inputs_wrong(&env));
        assert!(!compound_zero_delta_wrong(&env));
        assert!(!wad_try_mul_negative_wrong(&env));
        assert!(!from_asset_7_wrong(&env));
    }
}
