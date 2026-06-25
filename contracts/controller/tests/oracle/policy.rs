use super::*;

#[test]
fn test_risk_increasing_rejects_every_loosening() {
    let p = OraclePolicy::RiskIncreasing;
    assert!(!p.allows_disabled_market());
    assert!(!p.allows_stale_source());
    assert!(!p.allows_unsafe_deviation());
    assert!(!p.allows_degraded_dual_source());
    assert!(!p.allows_sanity_violation());
}

#[test]
fn test_risk_decreasing_permits_stale_and_fallback() {
    let p = OraclePolicy::RiskDecreasing;
    assert!(!p.allows_disabled_market());
    assert!(p.allows_stale_source());
    assert!(p.allows_unsafe_deviation());
    assert!(p.allows_degraded_dual_source());
    // Supply only reduces risk, so a misconfigured/out-of-bounds price
    // must not block it; value extraction requires a risk-increasing flow.
    assert!(p.allows_sanity_violation());
}

#[test]
fn test_repay_permits_disabled_and_stale() {
    let p = OraclePolicy::Repay;
    assert!(p.allows_disabled_market());
    assert!(p.allows_stale_source());
    assert!(p.allows_unsafe_deviation());
    assert!(p.allows_degraded_dual_source());
    assert!(p.allows_sanity_violation());
}

#[test]
fn test_liquidation_rejects_every_loosening() {
    let p = OraclePolicy::Liquidation;
    assert!(!p.allows_disabled_market());
    assert!(!p.allows_stale_source());
    // Liquidation rejects PrimaryWithAnchor divergence beyond the last
    // tolerance band; unsafe deviation is tolerated only on single-source paths.
    assert!(!p.allows_unsafe_deviation());
    assert!(!p.allows_degraded_dual_source());
    // Seizure sizing must read a sanity-checked price.
    assert!(!p.allows_sanity_violation());
}

#[test]
fn test_view_permits_everything() {
    let p = OraclePolicy::View;
    assert!(p.allows_disabled_market());
    assert!(p.allows_stale_source());
    assert!(p.allows_unsafe_deviation());
    assert!(p.allows_degraded_dual_source());
    assert!(p.allows_sanity_violation());
}

// In-crate (no external harness) so they run on each `cargo test -p
// controller`, giving fast regression signal on the policy matrix.

#[test]
fn test_liquidation_matches_risk_increasing_allowances() {
    // Liquidation and RiskIncreasing have identical allowance tables; they
    // stay distinct for intent/auditing. Guards against silent drift.
    let liq = OraclePolicy::Liquidation;
    let inc = OraclePolicy::RiskIncreasing;
    assert_eq!(liq.allows_disabled_market(), inc.allows_disabled_market());
    assert_eq!(liq.allows_stale_source(), inc.allows_stale_source());
    assert_eq!(liq.allows_unsafe_deviation(), inc.allows_unsafe_deviation());
    assert_eq!(
        liq.allows_degraded_dual_source(),
        inc.allows_degraded_dual_source()
    );
    assert_eq!(liq.allows_sanity_violation(), inc.allows_sanity_violation());
}

#[test]
fn test_repay_and_view_share_fully_permissive_allowances() {
    // Repay and View share the most-lenient allowances by design: Repay so
    // debt-reduction succeeds on degraded oracles, View for read-only
    // queries. Distinct types preserve intent for auditing.
    assert_eq!(
        OraclePolicy::Repay.allows_disabled_market(),
        OraclePolicy::View.allows_disabled_market()
    );
    assert_eq!(
        OraclePolicy::Repay.allows_degraded_dual_source(),
        OraclePolicy::View.allows_degraded_dual_source()
    );
    assert_eq!(
        OraclePolicy::Repay.allows_sanity_violation(),
        OraclePolicy::View.allows_sanity_violation()
    );
}
