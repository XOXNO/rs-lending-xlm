use super::*;
use crate::constants::{MAX_BORROW_RATE_RAY, MILLISECONDS_PER_YEAR, RAY, SUPPLY_INDEX_FLOOR_RAW};
use crate::rates::compound::{compound_interest, MAX_COMPOUND_DELTA_MS};
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

    let expected = RAY * 105 / 100;
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
fn test_cap_still_backstops_extreme_reward() {
    let env = Env::default();

    let supplied = Ray::from_asset(1, 7);
    let reward = Ray::from(i128::MAX / 2);

    let grown = update_supply_index(&env, supplied, Ray::from(RAY), reward);

    assert_eq!(grown.raw(), MAX_SUPPLY_INDEX_RAY);
}

#[test]
fn test_supply_index_reward_distributes_full_reward_without_offset() {
    let env = Env::default();
    let supplied = Ray::from(1_000 * RAY);
    let rewards = Ray::from(10 * RAY);

    let grown = update_supply_index(&env, supplied, Ray::from(RAY), rewards);

    let expected = RAY + RAY * 10 / 1000;
    assert!((grown.raw() - expected).abs() <= 1);
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

const SWEEP_SUPPLIED_MAX: i128 = i128::MAX / (2 * (MAX_SUPPLY_INDEX_RAY / RAY));

const LADDER_MAX: usize = 128;

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
        MAX_SUPPLY_INDEX_RAY - 1,
        MAX_SUPPLY_INDEX_RAY,
    ];

    for &supplied in &supplied_vals[..supplied_len] {
        for old_index in index_vals[..index_len].iter().chain(index_specials.iter()) {
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
            let denom = total_supplied_value;

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

    let mut state: u64 = 0x0BAD_C0FF_EE0D_D00D;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

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

    for _ in 0..60_000 {
        let supplied = between(1, SWEEP_SUPPLIED_MAX);
        let old_index = between(SUPPLY_INDEX_FLOOR_RAW, 100_000 * RAY);
        let rewards = between(0, 1_000_000 * RAY);
        assert_reward_is_conserved(&env, supplied, old_index, rewards);
    }
}

#[test]
#[should_panic(expected = "#33")]
fn test_supply_index_shortfall_requires_index_within_cap() {
    let env = Env::default();

    let supplied = Ray::from(RAY);
    let old_index = Ray::from(2 * MAX_SUPPLY_INDEX_RAY);
    let rewards = Ray::from(RAY);

    let new_index = update_supply_index(&env, supplied, old_index, rewards);
    assert_eq!(new_index.raw(), MAX_SUPPLY_INDEX_RAY);

    let _ = supply_index_reward_shortfall(&env, supplied, old_index, new_index, rewards);
}

fn one_token_value(decimals: u32) -> Ray {
    Ray::from_asset(10i128.pow(decimals), decimals)
}

#[test]
fn test_one_whole_token_normalizes_to_one_ray_at_every_decimals() {
    for decimals in [0u32, 6, 7, 8, 18] {
        assert_eq!(
            one_token_value(decimals).raw(),
            RAY,
            "one whole token must be exactly RAY of value at {decimals} decimals",
        );
    }
}

// --- index ceiling reachability (docs/reference/numeric-bounds.md §2) -----
//
// `MAX_BORROW_INDEX_RAY` is a clamp, not an overflow guard: at the ceiling the
// borrow index stops moving and debt stops accruing, silently. The tests below
// pin how far away that is, so a rate-cap or chunking change that brings it
// closer has to move a number here.

/// Fastest possible growth: every chunk is the largest `global_sync` will take
/// (`MAX_COMPOUND_DELTA_MS`, one year) at a pinned 100% utilization, so the
/// borrow rate sits at `annual_rate_ray` for the whole span.
fn max_chunk_growth_factor(env: &Env, annual_rate_ray: i128) -> Ray {
    let rate_per_ms = Ray::from(annual_rate_ray).div_by_int(MILLISECONDS_PER_YEAR as i128);
    compound_interest(env, rate_per_ms, MAX_COMPOUND_DELTA_MS)
}

/// Number of maximum-size compound chunks needed to drive the borrow index from
/// its `create_market` value of one RAY to `MAX_BORROW_INDEX_RAY`.
fn max_chunks_to_borrow_index_ceiling(env: &Env, annual_rate_ray: i128) -> u32 {
    let factor = max_chunk_growth_factor(env, annual_rate_ray);
    assert!(
        factor > Ray::ONE,
        "a non-growing factor would never reach the ceiling"
    );

    let mut index = Ray::ONE;
    let mut chunks = 0u32;
    while index.raw() < MAX_BORROW_INDEX_RAY {
        index = update_borrow_index(env, index, factor);
        chunks += 1;
        assert!(chunks < 10_000, "growth stalled below the ceiling");
    }
    chunks
}

#[test]
fn test_borrow_index_ceiling_is_eleven_years_away_at_the_protocol_rate_cap() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();

    // MAX_BORROW_RATE_RAY is 200% APR, the highest `MarketParamsRaw::validate`
    // will accept for `max_borrow_rate`.
    assert_eq!(
        max_chunks_to_borrow_index_ceiling(&env, MAX_BORROW_RATE_RAY),
        11,
        "the borrow index ceiling must stay more than a decade out at the rate cap",
    );
}

#[test]
fn test_borrow_index_ceiling_years_at_configured_and_realistic_rates() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();

    // 175% and 125% are the two `max_borrow_rate` values in configs/mainnet.
    assert_eq!(
        max_chunks_to_borrow_index_ceiling(&env, RAY * 175 / 100),
        12
    );
    assert_eq!(
        max_chunks_to_borrow_index_ceiling(&env, RAY * 125 / 100),
        17
    );
    // ChainSecurity's own worked example for Aave's uint120 `drawnIndex`.
    assert_eq!(max_chunks_to_borrow_index_ceiling(&env, RAY * 30 / 100), 70);
    assert_eq!(max_chunks_to_borrow_index_ceiling(&env, RAY / 10), 208);
}

#[test]
fn test_borrow_index_at_the_ceiling_multiplies_without_overflow() {
    let env = Env::default();

    // `update_borrow_index` multiplies before it clamps, so the pre-clamp
    // product at the ceiling times the largest reachable chunk factor is the
    // real overflow site. It must stay inside i128 with room to spare.
    let factor = max_chunk_growth_factor(&env, MAX_BORROW_RATE_RAY);
    let at_ceiling = Ray::from(MAX_BORROW_INDEX_RAY);

    let product = at_ceiling.mul(&env, factor);
    assert!(product.raw() > MAX_BORROW_INDEX_RAY);
    assert!(
        product.raw() < i128::MAX / 20,
        "pre-clamp headroom above the ceiling fell below 20x: {}",
        product.raw()
    );

    assert_eq!(
        update_borrow_index(&env, at_ceiling, factor).raw(),
        MAX_BORROW_INDEX_RAY,
    );
}

#[test]
fn test_supply_index_shares_the_borrow_index_ceiling() {
    assert_eq!(MAX_SUPPLY_INDEX_RAY, MAX_BORROW_INDEX_RAY);
    // Both indexes start at one RAY, so the ceiling is a 1e9x growth budget.
    assert_eq!(MAX_BORROW_INDEX_RAY / RAY, 1_000_000_000);
    // And the floor is 1e-3, so the supply index spans twelve decades.
    assert_eq!(RAY / SUPPLY_INDEX_FLOOR_RAW, 1_000);
}
