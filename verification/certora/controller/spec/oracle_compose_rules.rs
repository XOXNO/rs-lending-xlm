/// Oracle compose and dual-source degradation rules.
///
/// Verifies `compose.rs` policy gates for `allows_degraded_dual_source`,
/// `fallback_to_primary`, and stale-anchor handling under pinned
/// `OraclePolicy` variants.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use crate::oracle::policy::OraclePolicy;
use crate::oracle::ResolvedOracleComponents;
use controller::constants::WAD;

#[rule]
fn strict_policies_forbid_degraded_dual_source() {
    for policy in [
        OraclePolicy::RiskIncreasing,
        OraclePolicy::Liquidation,
        OraclePolicy::IsolatedRepay,
    ] {
        cvlr_assert!(!policy.allows_degraded_dual_source());
    }
    cvlr_satisfy!(true);
}

#[rule]
fn permissive_policies_allow_degraded_dual_source() {
    for policy in [
        OraclePolicy::RiskDecreasing,
        OraclePolicy::Repay,
        OraclePolicy::View,
    ] {
        cvlr_assert!(policy.allows_degraded_dual_source());
    }
    cvlr_satisfy!(true);
}

/// `fallback_to_primary` under a permissive policy returns the primary feed
/// unchanged and clears the anchor leg.
#[rule]
fn fallback_to_primary_succeeds_under_repay_policy(e: Env, primary_price: i128, timestamp: u64) {
    cvlr_assume!(primary_price > 0 && primary_price <= 1_000_000 * WAD);
    cvlr_assume!(timestamp > 0);

    let cache = crate::cache::Cache::new(&e, OraclePolicy::Repay);
    let resolved = crate::oracle::certora::fallback_to_primary(&cache, primary_price, timestamp);

    cvlr_assert!(resolved.final_price_wad == primary_price);
    cvlr_assert!(resolved.primary_price_wad == Some(primary_price));
    cvlr_assert!(resolved.anchor_price_wad.is_none());
    cvlr_assert!(!resolved.within_first_tolerance);
    cvlr_assert!(!resolved.within_second_tolerance);
}

/// Strict flows must revert when dual-source resolution degrades to primary-only.
#[rule]
fn fallback_to_primary_panics_under_liquidation_policy(
    e: Env,
    primary_price: i128,
    timestamp: u64,
) {
    cvlr_assume!(primary_price > 0 && primary_price <= 1_000_000 * WAD);
    cvlr_assume!(timestamp > 0);

    let cache = crate::cache::Cache::new(&e, OraclePolicy::Liquidation);
    let _resolved: ResolvedOracleComponents =
        crate::oracle::certora::fallback_to_primary(&cache, primary_price, timestamp);

    cvlr_satisfy!(false);
}

/// Stale anchors are treated as unusable under permissive policies without panic.
#[rule]
fn anchor_stale_unusable_under_repay_policy(e: Env, max_stale: u64) {
    cvlr_assume!((60..=86_400).contains(&max_stale));

    let cache = crate::cache::Cache::new(&e, OraclePolicy::Repay);
    let now = cache.ledger_timestamp_secs();
    let stale_ts = now.saturating_sub(max_stale.saturating_add(10));
    let observation = crate::oracle::certora::observation(WAD, stale_ts);

    let usable = crate::oracle::certora::anchor_is_usable(&cache, &observation, max_stale);

    cvlr_assert!(!usable);
}

/// Strict policies panic on stale anchor feeds instead of degrading silently.
#[rule]
fn anchor_stale_panics_under_liquidation_policy(e: Env, max_stale: u64) {
    cvlr_assume!((60..=86_400).contains(&max_stale));

    let cache = crate::cache::Cache::new(&e, OraclePolicy::Liquidation);
    let now = cache.ledger_timestamp_secs();
    let stale_ts = now.saturating_sub(max_stale.saturating_add(10));
    let observation = crate::oracle::certora::observation(WAD, stale_ts);

    let _usable = crate::oracle::certora::anchor_is_usable(&cache, &observation, max_stale);

    cvlr_satisfy!(false);
}

#[rule]
fn oracle_compose_reachability(e: Env, primary_price: i128) {
    cvlr_assume!(primary_price > 0 && primary_price <= WAD * 1000);
    let cache = crate::cache::Cache::new(&e, OraclePolicy::View);
    let _ = crate::oracle::certora::fallback_to_primary(&cache, primary_price, 1);
    cvlr_satisfy!(true);
}
