//! Index growth, reward distribution, and the protocol's cut.

use super::*;
use crate::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
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
    // factor = 1 + 1 ulp → product > MAX → clamp.
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
    // Growth = rewards / (supplied_value + virtual offset) = 5 / 101.
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
    // 1 * 1 / RAY == 0, so total_supplied_value is zero despite nonzero rewards.
    let out = update_supply_index(&env, Ray::from(1), Ray::from(1), Ray::from(5 * RAY));
    assert_eq!(out, Ray::from(1));
}

#[test]
fn test_supply_index_shortfall_accounts_full_reward() {
    let env = Env::default();
    // Funded market: 1,000 tokens (7dp) supplied at index RAY, 100-token reward.
    let supplied = Ray::from_asset(1_000, 7);
    let old_index = Ray::from(RAY);
    let reward = Ray::from_asset(100, 7);

    let new_index = update_supply_index(&env, supplied, old_index, reward);
    let shortfall = supply_index_reward_shortfall(&env, supplied, old_index, new_index, reward);
    let distributed = supplied
        .mul(&env, new_index)
        .checked_sub(&env, supplied.mul(&env, old_index));

    // 100% accounted: suppliers (via index) + protocol (shortfall) == full reward.
    assert_eq!(
        distributed.checked_add(&env, shortfall),
        reward,
        "distributed + shortfall must equal the full reward (no dead reserve)"
    );
    // The virtual offset genuinely under-distributes, so the shortfall is positive
    // and suppliers keep only their diluted (dust-safe) share.
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

/// Dust supply + large reward: index grows but stays below the cap.
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

/// Bounded index still accepts a later ordinary accrual.
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

/// Extreme reward still clamps at `MAX_SUPPLY_INDEX_RAY`.
#[test]
fn test_cap_still_backstops_extreme_reward() {
    let env = Env::default();

    let supplied = Ray::from_asset(1, 7);
    let reward = Ray::from(i128::MAX / 2);

    let grown = update_supply_index(&env, supplied, Ray::from(RAY), reward);

    assert_eq!(grown.raw(), MAX_SUPPLY_INDEX_RAY);
}

/// Funded market: offset dilutes growth by less than 1%.
#[test]
fn test_virtual_offset_negligible_for_funded_market() {
    let env = Env::default();
    let supplied = Ray::from(1_000 * RAY); // 1000 tokens
    let rewards = Ray::from(10 * RAY); // 1% reward

    let grown = update_supply_index(&env, supplied, Ray::from(RAY), rewards);

    // 1 + 10/1001 with offset; 1 + 10/1000 without.
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
    // In-range results are byte-identical to the conservative floor divide.
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
    // Post-wipeout floored index: the plain divide would push the share count past
    // i128 and trap. The overflow-safe form saturates, then caps to supply headroom.
    let supply_index = Ray::from(SUPPLY_INDEX_FLOOR_RAW);
    let fee = Ray::from(i128::MAX / 100);
    let supplied = Ray::from(1_000 * RAY);
    let shares = protocol_fee_shares(&env, fee, supply_index, supplied);
    assert_eq!(shares.raw(), i128::MAX - supplied.raw());
}

/// Proof for the surviving hypothesis: the single-shot virtual-offset defense in
/// `update_supply_index` does NOT bound growth when the SAME dust market is fed
/// many reward legs in sequence (as `Controller::add_rewards` does, one
/// load->update->save per non-deduplicated Vec leg). Each leg reloads the
/// persisted index, so growth COMPOUNDS across legs. With a 1-raw-unit seed on a
/// 7-decimal asset, ~30 modest legs (each ~doubling the index) drive `supply_index`
/// to the sticky `MAX_SUPPLY_INDEX_RAY` clamp for a modest total reward outlay,
/// after which ALL supplier yield is permanently discarded.
#[test]
fn test_iterated_reward_legs_pin_supply_index_and_yield_zero() {
    let env = Env::default();

    // Attacker seeds 1 raw unit of a 7-decimal asset (dust: 1e-7 tokens of value).
    let supplied = Ray::from_asset(1, 7);
    let mut index = Ray::from(RAY);

    // Walk the market by feeding legs that each roughly DOUBLE the index: reward =
    // (total_supplied_value + virtual_offset), i.e. exactly the reward denominator,
    // so factor = 1 + denom/denom = 2. This is the small-step regime the offset was
    // meant to bound; iterated it compounds geometrically.
    let mut total_reward_raw: i128 = 0;
    let mut legs = 0u32;
    while index.raw() < MAX_SUPPLY_INDEX_RAY && legs < 40 {
        let tsv = supplied.mul(&env, index).raw();
        let reward_raw = tsv + SUPPLY_VIRTUAL_VALUE_RAY; // == denom -> factor 2
        total_reward_raw = total_reward_raw.saturating_add(reward_raw);
        index = update_supply_index(&env, supplied, index, Ray::from(reward_raw));
        legs += 1;
    }

    // EXPLOIT ASSERTION 1: iterated legs pin the index at the sticky clamp.
    assert_eq!(
        index.raw(),
        MAX_SUPPLY_INDEX_RAY,
        "iterated add_rewards legs must pin supply_index at MAX",
    );
    assert!(legs <= 31, "cap reached in ~30 modest legs, got {legs}");

    // Total reward outlay stays modest (~hundreds of whole tokens on a 7-dp asset):
    // final leg cost ~= offset-in-tokens at the cap (~100 tokens), most recoverable
    // by the sole supplier's own withdraw. Net cost is a small stranded remainder.
    let total_reward_tokens = total_reward_raw / RAY; // whole tokens
    assert!(
        total_reward_tokens < 1_000,
        "total reward outlay to pin the market is modest ({total_reward_tokens} tokens)",
    );

    // EXPLOIT ASSERTION 2: with the index pinned, an ordinary later supplier-reward
    // accrual (real borrow interest, sized as tokens) is silently DISCARDED — the
    // clamp re-applies and the index does not move. Supplier yield is 0% forever.
    let ordinary_reward = Ray::from_asset(1_000, 7); // 1000 tokens of real interest
    let after = update_supply_index(&env, supplied, index, ordinary_reward);
    assert_eq!(
        after.raw(),
        MAX_SUPPLY_INDEX_RAY,
        "post-pin, real supplier interest is clamped away: index unchanged (0% yield)",
    );
}
