use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use common::oracle::observation::{
    check_not_future_at, is_future_at, is_stale, MAX_FUTURE_SKEW_SECONDS,
};

// Every operand in this module is a `u64` timestamp or age compared and added,
// never multiplied or divided, so there is no nonlinear term to bound and no
// native/widened branch to split. The `u64` type bound is the only ceiling the
// production functions have as well.

#[rule]
fn exact_staleness_boundary_is_fresh(now: u64, max_stale: u64) {
    cvlr_assume!(now >= max_stale);
    let observed_at = now - max_stale;
    cvlr_assert!(!is_stale(now, observed_at, max_stale));
}

#[rule]
fn one_second_past_staleness_boundary_is_stale(now: u64, max_stale: u64) {
    cvlr_assume!(max_stale < u64::MAX);
    cvlr_assume!(now > max_stale);
    let observed_at = now - max_stale - 1;
    cvlr_assert!(is_stale(now, observed_at, max_stale));
}

#[rule]
fn timestamp_at_future_skew_boundary_is_allowed(e: Env, now: u64) {
    cvlr_assume!(now <= u64::MAX - MAX_FUTURE_SKEW_SECONDS);
    check_not_future_at(&e, now, now + MAX_FUTURE_SKEW_SECONDS);
    cvlr_assert!(true);
}

/// One second past the skew allowance is rejected.
///
/// Revert shape: the trailing assert is reachable only if `check_not_future_at`
/// returns. Paired with `timestamp_beyond_future_skew_reverts_fixture_completes`.
#[rule]
fn timestamp_beyond_future_skew_reverts(e: Env, now: u64) {
    cvlr_assume!(now < u64::MAX - MAX_FUTURE_SKEW_SECONDS);
    check_not_future_at(&e, now, now + MAX_FUTURE_SKEW_SECONDS + 1);
    cvlr_assert!(false);
}

/// Satisfy twin of [`timestamp_beyond_future_skew_reverts`]: the same `now`
/// domain with the gate condition flipped from `now + SKEW + 1` to exactly
/// `now + SKEW`, so `check_not_future_at` returns instead of panicking.
///
/// [`timestamp_at_future_skew_boundary_is_allowed`] is the assert-form
/// counterpart of this witness and shares the same conf; this rule is the
/// satisfy form, which is what a `rule_sanity: none` revert conf needs beside
/// it once the vacuity check is turned on elsewhere.
#[rule]
fn timestamp_beyond_future_skew_reverts_fixture_completes(e: Env, now: u64) {
    cvlr_assume!(now < u64::MAX - MAX_FUTURE_SKEW_SECONDS);
    check_not_future_at(&e, now, now + MAX_FUTURE_SKEW_SECONDS);
    cvlr_satisfy!(!is_future_at(now, now + MAX_FUTURE_SKEW_SECONDS));
}
