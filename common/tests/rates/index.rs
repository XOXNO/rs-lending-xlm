use super::*;
use crate::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW, SUPPLY_INDEX_REWARD_CEILING_RAY};
use crate::rates::test_support::*;
use soroban_sdk::Env;

#[test]
fn test_update_borrow_index() {
    let env = Env::default();
    let old_index = Ray::ONE;
    let factor = Ray::from(RAY + RAY * 5 / 100);
    let new_index = update_borrow_index(&env, old_index, factor);
    let expected = RAY * 105 / 100;
    assert!((new_index.raw() - expected).abs() <= 1);
}

#[test]
fn test_update_borrow_index_at_max_does_not_panic() {
    let env = Env::default();
    let old_index = Ray::from(MAX_BORROW_INDEX_RAY);
    let new_index = update_borrow_index(&env, old_index, Ray::ONE);
    assert_eq!(new_index.raw(), MAX_BORROW_INDEX_RAY);
}

#[test]
fn test_update_borrow_index_above_max_clamps() {
    let env = Env::default();
    let old_index = Ray::from(MAX_BORROW_INDEX_RAY);

    let factor = Ray::from(RAY + 1);
    let new_index = update_borrow_index(&env, old_index, factor);
    assert_eq!(new_index.raw(), MAX_BORROW_INDEX_RAY);
}

#[test]
fn test_update_supply_index() {
    let env = Env::default();
    let supplied = Ray::from(100 * RAY);
    let old_index = Ray::ONE;
    let rewards = Ray::from(5 * RAY);
    let new_index = update_supply_index(&env, supplied, old_index, rewards);

    let expected = RAY * 106 / 101;
    assert!((new_index.raw() - expected).abs() <= 1);
}

#[test]
fn test_update_supply_index_zero_supplied() {
    let env = Env::default();
    let result = update_supply_index(&env, Ray::ZERO, Ray::ONE, Ray::from(5 * RAY));
    assert_eq!(result, Ray::ONE);
}

#[test]
fn test_update_supply_index_rounds_supplied_value_to_zero_returns_old_index() {
    let env = Env::default();

    let out = update_supply_index(&env, Ray::from(1), Ray::from(1), Ray::from(5 * RAY));
    assert_eq!(out, Ray::from(1));
}

#[test]
fn test_supply_index_shortfall_accounts_full_reward() {
    let env = Env::default();

    let supplied = Ray::from_asset(1_000, 7);
    let old_index = Ray::from(RAY);
    let reward = Ray::from_asset(100, 7);

    let new_index = update_supply_index(&env, supplied, old_index, reward);
    let shortfall = supply_index_reward_shortfall(&env, supplied, old_index, new_index, reward);
    let distributed = supplied
        .mul(&env, new_index)
        .checked_sub(&env, supplied.mul(&env, old_index));

    assert_eq!(
        distributed.checked_add(&env, shortfall),
        reward,
        "distributed + shortfall must equal the full reward (no dead reserve)"
    );

    assert!(
        shortfall.raw() > 0,
        "offset must leave a positive shortfall"
    );
    assert!(
        distributed.raw() > 0 && distributed.raw() < reward.raw(),
        "suppliers receive the diluted share, strictly less than the full reward"
    );
}

#[test]
fn test_supply_index_shortfall_high_valid_index_stays_conservative() {
    let env = Env::default();
    let supplied = Ray::from(100 * RAY);
    let old_index = Ray::from(145_000_436 * RAY);
    let reward = Ray::from_asset(1, 7);

    let new_index = update_supply_index(&env, supplied, old_index, reward);
    let distributed = supplied
        .mul(&env, new_index)
        .checked_sub(&env, supplied.mul(&env, old_index));
    assert!(distributed.raw() <= reward.raw());

    let shortfall = supply_index_reward_shortfall(&env, supplied, old_index, new_index, reward);
    assert_eq!(distributed.checked_add(&env, shortfall), reward);
}

#[test]
fn test_calculate_supplier_rewards() {
    let env = Env::default();
    let params = make_test_params(&env);

    let borrowed = Ray::from(100 * RAY);
    let old_index = Ray::ONE;
    let new_index = Ray::from(RAY + RAY / 100);

    let (rewards, fee) = calculate_supplier_rewards(&env, &params, borrowed, new_index, old_index);

    let expected_fee = RAY / 10;
    let expected_rewards = RAY * 9 / 10;

    assert!(
        (fee.raw() - expected_fee).abs() <= 1,
        "fee={}, expected={}",
        fee.raw(),
        expected_fee
    );
    assert!(
        (rewards.raw() - expected_rewards).abs() <= 1,
        "rewards={}, expected={}",
        rewards.raw(),
        expected_rewards
    );
}

#[test]
fn test_virtual_offset_bounds_dust_reward_growth() {
    let env = Env::default();

    let supplied = Ray::from_asset(1, 7);
    let reward = Ray::from_asset(170_141_183_459, 7);

    let grown = update_supply_index(&env, supplied, Ray::from(RAY), reward);

    assert!(grown.raw() > RAY, "reward must still grow the index");
    assert!(
        grown.raw() < MAX_SUPPLY_INDEX_RAY,
        "offset must keep growth below the cap"
    );
    assert!(
        grown.raw() < RAY * 1_000_000,
        "growth is bounded to ~1.7e31"
    );
}

#[test]
fn test_offset_supply_index_survives_ordinary_accrual() {
    let env = Env::default();

    let grown = update_supply_index(
        &env,
        Ray::from_asset(1, 7),
        Ray::from(RAY),
        Ray::from_asset(170_141_183_459, 7),
    );
    assert!(grown.raw() < MAX_SUPPLY_INDEX_RAY);

    let next = update_supply_index(&env, Ray::from(1), grown, Ray::from(170_000));
    assert!(next.raw() >= grown.raw());
    assert!(next.raw() < MAX_SUPPLY_INDEX_RAY);
}

#[test]
fn test_cap_still_backstops_extreme_reward() {
    let env = Env::default();

    let supplied = Ray::from_asset(1, 7);
    let reward = Ray::from(i128::MAX / 2);

    let grown = update_supply_index(&env, supplied, Ray::from(RAY), reward);

    assert_eq!(grown.raw(), MAX_SUPPLY_INDEX_RAY);
}

#[test]
fn test_virtual_offset_negligible_for_funded_market() {
    let env = Env::default();
    let supplied = Ray::from(1_000 * RAY);
    let rewards = Ray::from(10 * RAY);

    let grown = update_supply_index(&env, supplied, Ray::from(RAY), rewards);

    let with_offset = RAY + RAY * 10 / 1001;
    let offset_free = RAY + RAY * 10 / 1000;
    assert!((grown.raw() - with_offset).abs() <= 1);

    let drift = offset_free - grown.raw();
    assert!(
        drift * 100 < offset_free - RAY,
        "dilution < 1% of reward growth"
    );
}

#[test]
fn test_protocol_fee_shares_matches_floor_divide_in_range() {
    let env = Env::default();
    let supply_index = Ray::from(2 * RAY);
    let fee = Ray::from(500 * RAY);
    let supplied = Ray::from(1_000_000 * RAY);

    assert_eq!(
        protocol_fee_shares(&env, fee, supply_index, supplied).raw(),
        fee.div_floor(&env, supply_index).raw(),
    );
}

#[test]
fn test_protocol_fee_shares_never_overcredits_high_decimal_fee() {
    let env = Env::default();
    let supply_index = Ray::from(2 * RAY);
    let fee = Ray::from(1);
    let supplied = Ray::from(100 * RAY);

    let shares = protocol_fee_shares(&env, fee, supply_index, supplied);
    assert_eq!(shares, Ray::ZERO);
    assert!(shares.mul_floor(&env, supply_index) <= fee);
}

#[test]
fn test_protocol_fee_shares_saturates_and_caps_at_floored_index() {
    let env = Env::default();

    let supply_index = Ray::from(SUPPLY_INDEX_FLOOR_RAW);
    let fee = Ray::from(i128::MAX / 100);
    let supplied = Ray::from(1_000 * RAY);
    let shares = protocol_fee_shares(&env, fee, supply_index, supplied);
    assert_eq!(shares.raw(), i128::MAX - supplied.raw());
}

#[test]
fn test_iterated_reward_legs_pin_supply_index_and_yield_zero() {
    let env = Env::default();

    let supplied = Ray::from_asset(1, 7);
    let mut index = Ray::from(RAY);

    let mut total_reward_raw: i128 = 0;
    let mut legs = 0u32;
    while index.raw() < MAX_SUPPLY_INDEX_RAY && legs < 40 {
        let tsv = supplied.mul(&env, index).raw();
        let reward_raw = tsv + SUPPLY_VIRTUAL_VALUE_RAY;
        total_reward_raw = total_reward_raw.saturating_add(reward_raw);
        index = update_supply_index(&env, supplied, index, Ray::from(reward_raw));
        legs += 1;
    }

    assert_eq!(
        index.raw(),
        MAX_SUPPLY_INDEX_RAY,
        "iterated add_rewards legs must pin supply_index at MAX",
    );
    assert!(legs <= 31, "cap reached in ~30 modest legs, got {legs}");

    let total_reward_tokens = total_reward_raw / RAY;
    assert!(
        total_reward_tokens < 1_000,
        "total reward outlay to pin the market is modest ({total_reward_tokens} tokens)",
    );

    let ordinary_reward = Ray::from_asset(1_000, 7);
    let after = update_supply_index(&env, supplied, index, ordinary_reward);
    assert_eq!(
        after.raw(),
        MAX_SUPPLY_INDEX_RAY,
        "post-pin, real supplier interest is clamped away: index unchanged (0% yield)",
    );
}

// ---------------------------------------------------------------------------
// Reward-conservation sweep.
//
// `update_supply_index` derives the new index with two *floor* roundings
// (`div_floor` then `mul_div_floor_saturating`), while
// `supply_index_reward_shortfall` re-measures what that index actually handed
// out with two *half-up* roundings (`Ray::mul` on both legs). The two
// measurements can disagree by up to 1 ulp, and `Ray::checked_sub` panics with
// `MathOverflow` on a negative result -- so an over-measurement is not a
// rounding nit, it bricks the market: every pool entrypoint runs
// `interest::global_sync` first, and the same call sits in the read-only
// `simulate_update_indexes_body`.
//
// The margin that makes this safe is `rewards * RAY / (total_supplied_value +
// RAY)` -- the same order as the 1-ulp error -- so it is pinned here by sweep
// rather than left to inspection.
// ---------------------------------------------------------------------------

/// Largest `supplied` that keeps `supplied * MAX_SUPPLY_INDEX_RAY / RAY` inside
/// `i128`, so the sweeps exercise the shortfall algebra rather than tripping the
/// unrelated fixed-point overflow ceiling.
const SWEEP_SUPPLIED_MAX: i128 = i128::MAX / (2 * (MAX_SUPPLY_INDEX_RAY / RAY));

const LADDER_MAX: usize = 128;

/// Geometric ladder over `1..=max` stepping by `mul / div`, carrying the `-1`
/// and `+1` neighbours of every rung so ulp boundaries are covered.
fn ladder(max: i128, mul: i128, div: i128) -> ([i128; LADDER_MAX], usize) {
    let mut out = [0i128; LADDER_MAX];
    let mut len = 0usize;
    let push = |out: &mut [i128; LADDER_MAX], len: &mut usize, v: i128| {
        if v >= 1 && v <= max && *len < LADDER_MAX && (*len == 0 || out[*len - 1] != v) {
            out[*len] = v;
            *len += 1;
        }
    };

    let mut v: i128 = 1;
    loop {
        push(&mut out, &mut len, v - 1);
        push(&mut out, &mut len, v);
        push(&mut out, &mut len, v + 1);

        let next = match v.checked_mul(mul) {
            Some(scaled) => scaled / div,
            None => break,
        };
        if next > max {
            break;
        }
        v = if next <= v { v + 1 } else { next };
    }
    push(&mut out, &mut len, max);

    (out, len)
}

/// The single property under test: the index move can never hand suppliers more
/// than `rewards`, and whatever it withholds is returned as the shortfall so the
/// reward is conserved (undistributed dust becomes protocol revenue, it does not
/// vanish).
fn assert_reward_is_conserved(env: &Env, supplied_raw: i128, old_raw: i128, rewards_raw: i128) {
    let supplied = Ray::from(supplied_raw);
    let old_index = Ray::from(old_raw);
    let rewards = Ray::from(rewards_raw);

    let new_index = update_supply_index(env, supplied, old_index, rewards);
    assert!(
        new_index.raw() >= old_raw,
        "index moved backwards: supplied={supplied_raw} old_index={old_raw} \
         rewards={rewards_raw} new_index={}",
        new_index.raw(),
    );

    let distributed = supplied
        .mul(env, new_index)
        .checked_sub(env, supplied.mul(env, old_index));
    assert!(
        distributed.raw() <= rewards_raw,
        "over-distributed by {}: supplied={supplied_raw} old_index={old_raw} \
         rewards={rewards_raw} new_index={} distributed={}",
        distributed.raw() - rewards_raw,
        new_index.raw(),
        distributed.raw(),
    );

    let shortfall = supply_index_reward_shortfall(env, supplied, old_index, new_index, rewards);
    assert_eq!(
        distributed.raw() + shortfall.raw(),
        rewards_raw,
        "reward not conserved: supplied={supplied_raw} old_index={old_raw} rewards={rewards_raw}",
    );
}

#[test]
fn test_supply_index_reward_is_conserved_across_structured_grid() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();

    let (supplied_vals, supplied_len) = ladder(SWEEP_SUPPLIED_MAX, 50, 1);
    let (index_vals, index_len) = ladder(MAX_SUPPLY_INDEX_RAY, 50, 1);
    let (reward_vals, reward_len) = ladder(i128::MAX / 4, 200, 1);

    let index_specials = [
        SUPPLY_INDEX_FLOOR_RAW,
        SUPPLY_INDEX_FLOOR_RAW - 1,
        SUPPLY_INDEX_FLOOR_RAW + 1,
        RAY,
        SUPPLY_INDEX_REWARD_CEILING_RAY,
        MAX_SUPPLY_INDEX_RAY - 1,
        MAX_SUPPLY_INDEX_RAY,
    ];

    for &supplied in &supplied_vals[..supplied_len] {
        for old_index in index_vals[..index_len].iter().chain(index_specials.iter()) {
            // Dust rewards are where `rewards_ratio` and `increment` both sit on
            // their 0 -> 1 transition, so sweep them densely.
            for rewards in 0..40i128 {
                assert_reward_is_conserved(&env, supplied, *old_index, rewards);
            }
            for &rewards in &reward_vals[..reward_len] {
                assert_reward_is_conserved(&env, supplied, *old_index, rewards);
            }
        }
    }
}

#[test]
fn test_supply_index_reward_is_conserved_at_rounding_knife_edges() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();

    let (supplied_vals, supplied_len) = ladder(SWEEP_SUPPLIED_MAX, 40, 1);
    let (index_vals, index_len) = ladder(MAX_SUPPLY_INDEX_RAY, 40, 1);

    for &supplied_raw in &supplied_vals[..supplied_len] {
        for &old_raw in &index_vals[..index_len] {
            let supplied = Ray::from(supplied_raw);
            let old_index = Ray::from(old_raw);

            let total_supplied_value = supplied.mul(&env, old_index).raw();
            if total_supplied_value == 0 {
                continue;
            }
            let denom = total_supplied_value + SUPPLY_VIRTUAL_VALUE_RAY;

            // Smallest `rewards` that drives `rewards_ratio` to k, and the
            // smallest that drives `increment` to m -- the exact points where
            // the floor roundings inside `update_supply_index` step.
            let mut edges = [0i128; 12];
            let mut edges_len = 0usize;
            let push_edge = |edges: &mut [i128; 12], len: &mut usize, ratio: i128| {
                if ratio <= 0 {
                    return;
                }
                let rewards = fp_core::mul_div_floor_saturating(&env, ratio, denom, RAY);
                if rewards < i128::MAX && *len < 12 {
                    edges[*len] = rewards;
                    *len += 1;
                }
            };

            for ratio in [1i128, 2, 3, 17] {
                push_edge(&mut edges, &mut edges_len, ratio);
            }
            for increment in [1i128, 2, 9] {
                let ratio = fp_core::mul_div_floor_saturating(&env, increment, RAY, old_raw);
                push_edge(&mut edges, &mut edges_len, ratio);
            }
            // The reward that walks `grown` onto the MAX_SUPPLY_INDEX_RAY clamp,
            // where `new_index` stops tracking the floor-derived `grown`.
            if old_raw < MAX_SUPPLY_INDEX_RAY {
                let ratio = fp_core::mul_div_floor_saturating(
                    &env,
                    MAX_SUPPLY_INDEX_RAY - old_raw,
                    RAY,
                    old_raw,
                );
                push_edge(&mut edges, &mut edges_len, ratio);
            }

            for &edge in &edges[..edges_len] {
                for delta in -3i128..=3 {
                    let rewards = edge.saturating_add(delta);
                    if rewards >= 0 {
                        assert_reward_is_conserved(&env, supplied_raw, old_raw, rewards);
                    }
                }
            }
        }
    }
}

#[test]
fn test_supply_index_reward_is_conserved_under_pseudorandom_sweep() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();

    // xorshift64, fixed seed: a failure reproduces exactly and the assertion
    // messages print the offending triple.
    let mut state: u64 = 0x0BAD_C0FF_EE0D_D00D;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    // Log-uniform draw in `1..=max`, so dust magnitudes stay well represented.
    let mut log_uniform = |max: i128| -> i128 {
        let bits = (next() % 126) as u32 + 1;
        let span = ((1i128 << bits) - 1).min(max);
        let wide = ((next() as u128) << 64) | next() as u128;
        (wide % (span as u128)) as i128 + 1
    };

    for _ in 0..120_000 {
        let supplied = log_uniform(SWEEP_SUPPLIED_MAX);
        let old_index = log_uniform(MAX_SUPPLY_INDEX_RAY);
        let rewards = log_uniform(i128::MAX / 4);
        assert_reward_is_conserved(&env, supplied, old_index, rewards);
    }
}

#[test]
fn test_supply_index_reward_is_conserved_in_realistic_band() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();

    let mut state: u64 = 0x5EED_1234_ABCD_9876;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut between = |lo: i128, hi: i128| -> i128 {
        let wide = ((next() as u128) << 64) | next() as u128;
        lo + (wide % ((hi - lo + 1) as u128)) as i128
    };

    // Indexes between the storage floor and the `distribute_reward` ceiling,
    // rewards at token scale, supply spanning the full range that keeps the
    // market's value inside `i128`.
    for _ in 0..60_000 {
        let supplied = between(1, SWEEP_SUPPLIED_MAX);
        let old_index = between(SUPPLY_INDEX_FLOOR_RAW, SUPPLY_INDEX_REWARD_CEILING_RAY);
        let rewards = between(0, 1_000_000 * RAY);
        assert_reward_is_conserved(&env, supplied, old_index, rewards);
    }
}

#[test]
#[should_panic(expected = "#33")]
fn test_supply_index_shortfall_requires_index_within_cap() {
    // Documented precondition: `old_index` must already satisfy the
    // MAX_SUPPLY_INDEX_RAY cap. Above it the cap clamps `new_index` *down*,
    // `distributed` goes negative and `Ray::checked_sub` panics. The pool never
    // reaches this state (markets open at RAY, every write goes through
    // `update_supply_index`'s clamp or the bad-debt path, which only reduces),
    // so this pins the boundary rather than describing reachable behaviour.
    let env = Env::default();

    let supplied = Ray::from(RAY);
    let old_index = Ray::from(2 * MAX_SUPPLY_INDEX_RAY);
    let rewards = Ray::from(RAY);

    let new_index = update_supply_index(&env, supplied, old_index, rewards);
    assert_eq!(new_index.raw(), MAX_SUPPLY_INDEX_RAY);

    let _ = supply_index_reward_shortfall(&env, supplied, old_index, new_index, rewards);
}
