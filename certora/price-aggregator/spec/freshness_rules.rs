//! Timestamp-boundary rules used by every oracle provider.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::Env;

use common::oracle::observation::{check_not_future_at, is_stale, MAX_FUTURE_SKEW_SECONDS};

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

#[rule]
fn timestamp_beyond_future_skew_reverts(e: Env, now: u64) {
    cvlr_assume!(now < u64::MAX - MAX_FUTURE_SKEW_SECONDS);
    check_not_future_at(&e, now, now + MAX_FUTURE_SKEW_SECONDS + 1);
    cvlr_assert!(false);
}
