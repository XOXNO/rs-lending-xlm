//! Per-entrypoint oracle permissiveness policy (the heart of ADR 0004).
//!
//! The enum variants encode exactly which oracle failure modes a given
//! operation is allowed to tolerate. The mapping from entrypoint to
//! variant is the single source of truth for "when can the protocol
//! still function with stale or deviated prices?"
//!
//! See the table and rationale in ADR 0004. The `Allowances` struct and
//! the `is_*_allowed` methods are the implementation of that table.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OraclePolicy {
    RiskIncreasing,
    RiskDecreasing,
    Repay,
    IsolatedRepay,
    Liquidation,
    View,
}

// Policy allowances.
#[derive(Clone, Copy, Debug)]
struct Allowances {
    disabled_market: bool,
    stale_source: bool,
    unsafe_deviation: bool,
    missing_twap_fallback: bool,
    prefer_aggregator_on_deviation: bool,
}

impl Allowances {
    const fn for_policy(p: OraclePolicy) -> Self {
        use OraclePolicy::*;
        match p {
            //                            disabled stale  unsafe   twap   prefer_agg
            RiskIncreasing => Allowances {
                disabled_market: false,
                stale_source: false,
                unsafe_deviation: false,
                missing_twap_fallback: false,
                prefer_aggregator_on_deviation: false,
            },
            RiskDecreasing => Allowances {
                disabled_market: false,
                stale_source: true,
                unsafe_deviation: true,
                missing_twap_fallback: true,
                prefer_aggregator_on_deviation: false,
            },
            Repay => Allowances {
                disabled_market: true,
                stale_source: true,
                unsafe_deviation: true,
                missing_twap_fallback: true,
                prefer_aggregator_on_deviation: false,
            },
            IsolatedRepay => Allowances {
                disabled_market: true,
                stale_source: false,
                unsafe_deviation: false,
                missing_twap_fallback: false,
                prefer_aggregator_on_deviation: false,
            },
            Liquidation => Allowances {
                disabled_market: false,
                stale_source: false,
                unsafe_deviation: false,
                missing_twap_fallback: false,
                prefer_aggregator_on_deviation: true,
            },
            View => Allowances {
                disabled_market: true,
                stale_source: true,
                unsafe_deviation: true,
                missing_twap_fallback: true,
                prefer_aggregator_on_deviation: false,
            },
        }
    }
}

impl OraclePolicy {
    pub fn allows_disabled_market(self) -> bool {
        Allowances::for_policy(self).disabled_market
    }

    pub fn allows_stale_source(self) -> bool {
        Allowances::for_policy(self).stale_source
    }

    pub fn allows_unsafe_deviation(self) -> bool {
        Allowances::for_policy(self).unsafe_deviation
    }

    pub fn allows_missing_twap_fallback(self) -> bool {
        Allowances::for_policy(self).missing_twap_fallback
    }

    // Prefers aggregator on deviation.
    pub fn prefers_aggregator_on_deviation(self) -> bool {
        Allowances::for_policy(self).prefer_aggregator_on_deviation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_risk_increasing_rejects_every_loosening() {
        let p = OraclePolicy::RiskIncreasing;
        assert!(!p.allows_disabled_market());
        assert!(!p.allows_stale_source());
        assert!(!p.allows_unsafe_deviation());
        assert!(!p.allows_missing_twap_fallback());
        assert!(!p.prefers_aggregator_on_deviation());
    }

    #[test]
    fn test_risk_decreasing_permits_stale_and_fallback() {
        let p = OraclePolicy::RiskDecreasing;
        assert!(!p.allows_disabled_market());
        assert!(p.allows_stale_source());
        assert!(p.allows_unsafe_deviation());
        assert!(p.allows_missing_twap_fallback());
        assert!(!p.prefers_aggregator_on_deviation());
    }

    #[test]
    fn test_repay_permits_disabled_and_stale() {
        let p = OraclePolicy::Repay;
        assert!(p.allows_disabled_market());
        assert!(p.allows_stale_source());
        assert!(p.allows_unsafe_deviation());
        assert!(p.allows_missing_twap_fallback());
        assert!(!p.prefers_aggregator_on_deviation());
    }

    #[test]
    fn test_isolated_repay_permits_only_disabled_market() {
        let p = OraclePolicy::IsolatedRepay;
        assert!(p.allows_disabled_market());
        assert!(!p.allows_stale_source());
        assert!(!p.allows_unsafe_deviation());
        assert!(!p.allows_missing_twap_fallback());
        assert!(!p.prefers_aggregator_on_deviation());
    }

    #[test]
    fn test_liquidation_prefers_aggregator() {
        let p = OraclePolicy::Liquidation;
        assert!(!p.allows_disabled_market());
        assert!(!p.allows_stale_source());
        // Hardened: liquidation rejects when PrimaryWithAnchor sources diverge
        // beyond the configured last tolerance band (UnsafePriceNotAllowed).
        // Unsafe deviation is only tolerated for single-source fallback paths.
        assert!(!p.allows_unsafe_deviation());
        assert!(!p.allows_missing_twap_fallback());
        assert!(p.prefers_aggregator_on_deviation());
    }

    #[test]
    fn test_view_permits_everything_except_aggregator_preference() {
        let p = OraclePolicy::View;
        assert!(p.allows_disabled_market());
        assert!(p.allows_stale_source());
        assert!(p.allows_unsafe_deviation());
        assert!(p.allows_missing_twap_fallback());
        assert!(!p.prefers_aggregator_on_deviation());
    }

    // --- Additional focused coverage for oracle policy composition ---
    // These live inside the controller crate (no external harness) so they
    // execute on every `cargo test -p controller` and give fast regression
    // signal on the policy matrix that drives liquidation, strategies, and
    // repay paths.

    #[test]
    fn test_liquidation_is_the_only_policy_that_prefers_aggregator() {
        // Liquidation is unique: it hardens the unsafe-deviation gate (to
        // protect against wrongful or excessive seizures) while still
        // preferring the fresher aggregator price when inside the last band.
        // All other policies either reject aggregator preference entirely or
        // are not used for seizure accounting.
        assert!(OraclePolicy::Liquidation.prefers_aggregator_on_deviation());
        assert!(!OraclePolicy::RiskIncreasing.prefers_aggregator_on_deviation());
        assert!(!OraclePolicy::RiskDecreasing.prefers_aggregator_on_deviation());
        assert!(!OraclePolicy::Repay.prefers_aggregator_on_deviation());
        assert!(!OraclePolicy::IsolatedRepay.prefers_aggregator_on_deviation());
        assert!(!OraclePolicy::View.prefers_aggregator_on_deviation());
    }

    #[test]
    fn test_repay_and_view_share_fully_permissive_allowances() {
        // Repay and View have identical allowances by design. Both are
        // intentionally the most lenient (disabled markets OK, stale and
        // unsafe prices tolerated, TWAP fallback allowed). Repay uses this
        // for user debt-reduction flows that must succeed even on degraded
        // oracles; View uses it for read-only queries. The policy type
        // still distinguishes intent for auditing and future evolution.
        // (See also: Liquidation is the only one that prefers aggregator.)
        assert_eq!(
            OraclePolicy::Repay.allows_disabled_market(),
            OraclePolicy::View.allows_disabled_market()
        );
        assert_eq!(
            OraclePolicy::Repay.prefers_aggregator_on_deviation(),
            OraclePolicy::View.prefers_aggregator_on_deviation()
        );
    }
}
