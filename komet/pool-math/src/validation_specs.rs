//! Exact acceptance domains of production parameter validators and oracle
//! timestamp predicates (Certora counterparts: `freshness_rules.rs`,
//! `config-rejections-reverts`). The specifications are independent integer
//! formulas over widened types.
//!
//! `*_accepts` claims: on the specified domain the validator returns normally.
//! `*_rejects` claims: outside it the validator never returns normally; the
//! harness returns `false` right after the call, so the proof requires every
//! path there to stop inside the validator (expectation `trap` in the
//! pipeline manifest), while inputs in the domain return `true`.

use common::{
    constants::{BPS, MAX_LIQUIDATION_TARGET_HF_WAD, WAD},
    oracle::observation::{self, MAX_FUTURE_SKEW_SECONDS, MAX_TWAP_RECORDS},
    validation,
};
use soroban_sdk::Env;

fn risk_ok(ltv: u32, threshold: u32, bonus: u32) -> bool {
    let (l, t, b) = (i128::from(ltv), i128::from(threshold), i128::from(bonus));
    t > l && t <= BPS && t * (BPS + b) <= BPS * BPS
}

pub fn risk_bounds_accepts(env: &Env, ltv: u32, threshold: u32, bonus: u32) -> bool {
    if !risk_ok(ltv, threshold, bonus) {
        return true;
    }
    validation::validate_risk_bounds(env, ltv, threshold, bonus);
    true
}

pub fn risk_bounds_rejects(env: &Env, ltv: u32, threshold: u32, bonus: u32) -> bool {
    if risk_ok(ltv, threshold, bonus) {
        return true;
    }
    validation::validate_risk_bounds(env, ltv, threshold, bonus);
    false
}

/// Deliberately false: the valid (7000, 8000, 500) is claimed to be rejected;
/// the validator returns normally, so the call completes with `false`.
pub fn risk_bounds_reject_wrong(env: &Env) -> bool {
    validation::validate_risk_bounds(env, 7_000, 8_000, 500);
    false
}

fn twap_ok(records: u32) -> bool {
    (1..=12).contains(&records)
}

pub fn twap_records_accepts(env: &Env, records: u32) -> bool {
    if !twap_ok(records) {
        return true;
    }
    validation::validate_twap_records(env, records);
    true
}

pub fn twap_records_rejects(env: &Env, records: u32) -> bool {
    if twap_ok(records) {
        return true;
    }
    validation::validate_twap_records(env, records);
    false
}

/// Deliberately false: 12 records are claimed to be rejected.
pub fn twap_records_reject_wrong(env: &Env) -> bool {
    validation::validate_twap_records(env, 12);
    false
}

fn curve_ok(target: i128, max_bonus_hf: i128, factor: u32) -> bool {
    target > WAD
        && target <= 10 * WAD
        && max_bonus_hf > 0
        && max_bonus_hf < target
        && factor > 0
        && i128::from(factor) <= BPS
}

pub fn liquidation_curve_accepts(env: &Env, target: i128, max_bonus_hf: i128, factor: u32) -> bool {
    if !curve_ok(target, max_bonus_hf, factor) {
        return true;
    }
    validation::validate_liquidation_curve(env, target, max_bonus_hf, factor);
    true
}

pub fn liquidation_curve_rejects(env: &Env, target: i128, max_bonus_hf: i128, factor: u32) -> bool {
    if curve_ok(target, max_bonus_hf, factor) {
        return true;
    }
    validation::validate_liquidation_curve(env, target, max_bonus_hf, factor);
    false
}

/// is_stale(now, feed, max) == (now > feed + max), over every u64 triple.
pub fn stale_exact(now: u64, feed: u64, max_stale: u64) -> bool {
    observation::is_stale(now, feed, max_stale)
        == (u128::from(now) > u128::from(feed) + u128::from(max_stale))
}

/// Deliberately false: a feed exactly max_stale old is not stale.
pub fn stale_wrong() -> bool {
    observation::is_stale(100, 40, 60)
}

/// is_future_at(now, feed) == (now + skew fits u64 and feed > now + skew).
pub fn future_exact(now: u64, feed: u64) -> bool {
    let limit = u128::from(now) + u128::from(MAX_FUTURE_SKEW_SECONDS);
    observation::is_future_at(now, feed)
        == (limit <= u128::from(u64::MAX) && u128::from(feed) > limit)
}

/// Deliberately false: a feed exactly at the skew boundary is not in the future.
pub fn future_wrong() -> bool {
    observation::is_future_at(1_000, 1_000 + MAX_FUTURE_SKEW_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_witnesses() {
        let env = Env::default();
        assert_eq!(MAX_TWAP_RECORDS, 12);
        assert_eq!(MAX_LIQUIDATION_TARGET_HF_WAD, 10 * WAD);
        assert!(risk_bounds_accepts(&env, 7_000, 8_000, 500));
        assert!(risk_bounds_accepts(&env, 0, 10_000, 0));
        assert!(risk_ok(0, 9_523, 500));
        assert!(!risk_ok(0, 9_524, 500));
        for r in 1..=12 {
            assert!(twap_records_accepts(&env, r));
        }
        assert!(liquidation_curve_accepts(&env, 2 * WAD, WAD, 1));
        for (n, f, m) in [
            (0, 0, 0),
            (100, 40, 60),
            (101, 40, 60),
            (u64::MAX, 0, u64::MAX - 1),
            (5, 9, 0),
        ] {
            assert!(stale_exact(n, f, m));
        }
        for (n, f) in [
            (0, 61),
            (0, 60),
            (u64::MAX, u64::MAX),
            (u64::MAX - 60, u64::MAX),
            (u64::MAX - 61, u64::MAX),
        ] {
            assert!(future_exact(n, f));
        }
        assert!(!stale_wrong());
        assert!(!future_wrong());
    }
}
