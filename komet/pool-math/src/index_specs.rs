//! Production index/accounting claims over explicit representable domains.
//! These are executable specifications, not completed Komet proofs.

use common::{
    constants::{BPS, MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW},
    math::fp::{Bps, Ray},
    rates,
    types::MarketParams,
};
use soroban_sdk::{Address, Env, I256};

/// For N >= 0 and D > 0, characterize clamp(floor(N / D), low, high)
/// by quotient/remainder inequalities, without calling production division.
fn clamped_floor(env: &Env, n: &I256, d: i128, low: i128, high: i128, q: i128) -> bool {
    let denominator = I256::from_i128(env, d);
    let quotient = I256::from_i128(env, q);
    q >= low
        && q <= high
        && (q == low || quotient.mul(&denominator) <= *n)
        && (q == high || *n < quotient.add(&I256::from_i128(env, 1)).mul(&denominator))
}

/// Independent exact-integer valuation; i128 nonnegative products fit I256.
fn value(env: &Env, shares: i128, index: i128) -> I256 {
    I256::from_i128(env, shares)
        .mul(&I256::from_i128(env, index))
        .add(&I256::from_i128(env, RAY / 2))
        .div(&I256::from_i128(env, RAY))
}

fn supply_domain(supplied: i128, old: i128, rewards: i128) -> bool {
    supplied >= 0 && (SUPPLY_INDEX_FLOOR_RAW..=MAX_SUPPLY_INDEX_RAY).contains(&old) && rewards >= 0
}

#[inline(never)]
fn borrow_index(env: &Env, old: i128, factor: i128) -> i128 {
    rates::update_borrow_index(env, Ray::from(old), Ray::from(factor)).raw()
}

#[inline(never)]
fn supply_index(env: &Env, supplied: i128, old: i128, rewards: i128) -> i128 {
    rates::update_supply_index(env, Ray::from(supplied), Ray::from(old), Ray::from(rewards)).raw()
}

/// Exact half-up growth followed by the cap. The pre-cap quotient must fit
/// i128: production multiplies before clamping. No arbitrary factor ceiling.
pub fn borrow_index_exact(env: &Env, old: i128, factor: i128) -> bool {
    if !(RAY..=MAX_BORROW_INDEX_RAY).contains(&old) || factor < RAY {
        return true;
    }
    let numerator = I256::from_i128(env, old)
        .mul(&I256::from_i128(env, factor))
        .add(&I256::from_i128(env, RAY / 2));
    let first_unrepresentable = I256::from_i128(env, i128::MAX)
        .add(&I256::from_i128(env, 1))
        .mul(&I256::from_i128(env, RAY));
    if numerator >= first_unrepresentable {
        return true;
    }
    let result = borrow_index(env, old, factor);
    result >= old && clamped_floor(env, &numerator, RAY, 0, MAX_BORROW_INDEX_RAY, result)
}

/// Exact early returns and capped floor growth. On the arithmetic branch,
/// rounded old value and old value + rewards must fit i128 before clamping.
pub fn supply_index_exact(env: &Env, supplied: i128, old: i128, rewards: i128) -> bool {
    if !supply_domain(supplied, old, rewards) {
        return true;
    }
    if supplied == 0 || rewards == 0 {
        return supply_index(env, supplied, old, rewards) == old;
    }
    let Some(old_value) = value(env, supplied, old).to_i128() else {
        return true;
    };
    if old_value == 0 {
        return supply_index(env, supplied, old, rewards) == old;
    }
    let Some(new_value) = old_value.checked_add(rewards) else {
        return true;
    };
    let numerator = I256::from_i128(env, new_value).mul(&I256::from_i128(env, RAY));
    let result = supply_index(env, supplied, old, rewards);
    clamped_floor(env, &numerator, supplied, old, MAX_SUPPLY_INDEX_RAY, result)
}

/// Distributed value + the actual production shortfall equals rewards.
/// Requires representable old value + rewards, including on zero-input paths.
pub fn supply_reward_conservation(env: &Env, supplied: i128, old: i128, rewards: i128) -> bool {
    if !supply_domain(supplied, old, rewards) {
        return true;
    }
    let Some(old_value) = value(env, supplied, old).to_i128() else {
        return true;
    };
    if old_value.checked_add(rewards).is_none() {
        return true;
    }
    let result = supply_index(env, supplied, old, rewards);
    let distributed = value(env, supplied, result).sub(&I256::from_i128(env, old_value));
    let reward = I256::from_i128(env, rewards);
    if distributed < I256::from_i128(env, 0) || distributed > reward {
        return false;
    }
    let shortfall = rates::supply_index_reward_shortfall(
        env,
        Ray::from(supplied),
        Ray::from(old),
        Ray::from(result),
        Ray::from(rewards),
    );
    shortfall.raw() >= 0 && distributed.add(&I256::from_i128(env, shortfall.raw())) == reward
}

/// Exact reserve fee and supplier remainder of independently valued debt.
/// Both debt valuations must fit i128; all nonnegative borrowed share counts
/// are otherwise admitted. Only reserve_factor affects this production helper.
pub fn interest_split_exact(
    env: &Env,
    asset: Address,
    borrowed: i128,
    old: i128,
    new: i128,
    reserve_bps: u32,
) -> bool {
    if borrowed < 0
        || !(RAY..=MAX_BORROW_INDEX_RAY).contains(&old)
        || new < old
        || new > MAX_BORROW_INDEX_RAY
        || i128::from(reserve_bps) >= BPS
    {
        return true;
    }
    let (Some(old_value), Some(new_value)) = (
        value(env, borrowed, old).to_i128(),
        value(env, borrowed, new).to_i128(),
    ) else {
        return true;
    };
    let accrued = new_value - old_value;
    let params = MarketParams {
        max_borrow_rate: Ray::ONE,
        base_borrow_rate: Ray::ZERO,
        slope1: Ray::from(RAY / 10),
        slope2: Ray::from(RAY / 10),
        slope3: Ray::from(RAY / 10),
        mid_utilization: Ray::from(RAY / 2),
        optimal_utilization: Ray::from(RAY * 4 / 5),
        max_utilization: Ray::ONE,
        reserve_factor: Bps::from(i128::from(reserve_bps)),
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: asset,
        asset_decimals: 7,
    };
    let (supplier, fee) = rates::calculate_supplier_rewards(
        env,
        &params,
        Ray::from(borrowed),
        Ray::from(new),
        Ray::from(old),
    );
    let numerator = I256::from_i128(env, accrued)
        .mul(&I256::from_i128(env, i128::from(reserve_bps)))
        .add(&I256::from_i128(env, BPS / 2));
    supplier.raw() >= 0
        && fee.raw() >= 0
        && I256::from_i128(env, supplier.raw()).add(&I256::from_i128(env, fee.raw()))
            == I256::from_i128(env, accrued)
        && clamped_floor(env, &numerator, BPS, 0, i128::MAX, fee.raw())
}

/// Exact floor fee-share mint capped by remaining supply headroom, over the
/// entire nonnegative i128 fee/supply domain. A cap can leave unbooked value.
pub fn fee_shares_exact(env: &Env, fee: i128, index: i128, supplied: i128) -> bool {
    if fee < 0 || supplied < 0 || !(SUPPLY_INDEX_FLOOR_RAW..=MAX_SUPPLY_INDEX_RAY).contains(&index)
    {
        return true;
    }
    let headroom = i128::MAX - supplied;
    let numerator = I256::from_i128(env, fee).mul(&I256::from_i128(env, RAY));
    let result =
        rates::protocol_fee_shares(env, Ray::from(fee), Ray::from(index), Ray::from(supplied))
            .raw();
    clamped_floor(env, &numerator, index, 0, headroom, result)
}

#[inline(never)]
fn flash_fee(env: &Env, amount: i128, fee_bps: u32) -> i128 {
    Bps::from(i128::from(fee_bps)).flash_loan_fee_on(env, amount)
}

/// Exact half-up fee with the positive-rate minimum of one. Includes zero
/// amounts and every u32 rate, provided the pre-minimum fee fits i128.
/// This proves fee arithmetic only, not the callback or repayment flow.
pub fn flash_fee_exact(env: &Env, amount: i128, fee_bps: u32) -> bool {
    if amount < 0 {
        return true;
    }
    let numerator = I256::from_i128(env, amount)
        .mul(&I256::from_i128(env, i128::from(fee_bps)))
        .add(&I256::from_i128(env, BPS / 2));
    let first_unrepresentable = I256::from_i128(env, i128::MAX)
        .add(&I256::from_i128(env, 1))
        .mul(&I256::from_i128(env, BPS));
    if numerator >= first_unrepresentable {
        return true;
    }
    let result = flash_fee(env, amount, fee_bps);
    clamped_floor(
        env,
        &numerator,
        BPS,
        i128::from(fee_bps > 0),
        i128::MAX,
        result,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_fee_rounding_minimum_and_widened_witnesses() {
        let env = Env::default();
        for (amount, bps, expected) in [
            (0, 0, 0),
            (0, 1, 1),
            (1, 1, 1),
            (4_999, 1, 1),
            (14_999, 1, 1),
            (15_000, 1, 2),
            (1, 10_000, 1),
            (1, u32::MAX, 429_497),
            (i128::MAX, 0, 0),
            (i128::MAX, 10_000, i128::MAX),
        ] {
            assert_eq!(flash_fee(&env, amount, bps), expected);
            assert!(flash_fee_exact(&env, amount, bps));
            let numerator = I256::from_i128(&env, amount)
                .mul(&I256::from_i128(&env, i128::from(bps)))
                .add(&I256::from_i128(&env, BPS / 2));
            let wrong = if expected == i128::MAX {
                expected - 1
            } else {
                expected + 1
            };
            assert!(!clamped_floor(
                &env,
                &numerator,
                BPS,
                i128::from(bps > 0),
                i128::MAX,
                wrong
            ));
        }
    }

    #[test]
    fn exact_claims_cover_rounding_and_saturation() {
        let env = Env::default();
        for (n, d, low, high) in [(0, 3, 0, 10), (14, 3, 5, 10), (29, 3, 0, 10), (99, 3, 5, 5)] {
            for q in 0..=11 {
                assert_eq!(
                    clamped_floor(&env, &I256::from_i128(&env, n), d, low, high, q),
                    q == (n / d).clamp(low, high)
                );
            }
        }
        assert!(borrow_index_exact(&env, RAY, RAY));
        assert!(borrow_index_exact(&env, MAX_BORROW_INDEX_RAY, 8 * RAY));
        for (supplied, old, rewards) in [
            (0, MAX_SUPPLY_INDEX_RAY, i128::MAX),
            (i128::MAX, MAX_SUPPLY_INDEX_RAY, 0),
            (1, SUPPLY_INDEX_FLOOR_RAW, 1),
            (1, RAY, 1),
            (RAY, RAY, RAY),
            (RAY / 10, MAX_SUPPLY_INDEX_RAY, RAY),
        ] {
            assert!(supply_index_exact(&env, supplied, old, rewards));
            assert!(supply_reward_conservation(&env, supplied, old, rewards));
        }
        assert!(fee_shares_exact(&env, i128::MAX, SUPPLY_INDEX_FLOOR_RAW, 0));
        assert!(fee_shares_exact(&env, RAY, RAY, i128::MAX));
        let asset = Address::from_string(&soroban_sdk::String::from_str(
            &env,
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM",
        ));
        for reserve in [0, 1, (BPS - 1) as u32] {
            assert!(interest_split_exact(
                &env,
                asset.clone(),
                RAY,
                RAY,
                2 * RAY,
                reserve
            ));
        }
    }
}
