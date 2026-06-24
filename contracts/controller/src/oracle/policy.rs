//! Per-entrypoint oracle failure policy.
//! Each controller flow chooses one variant before reading prices.
//! The variant decides which oracle failures the flow may tolerate.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OraclePolicy {
    RiskIncreasing,
    RiskDecreasing,
    Repay,
    Liquidation,
    View,
}

#[derive(Clone, Copy, Debug)]
struct Allowances {
    disabled_market: bool,
    stale_source: bool,
    unsafe_deviation: bool,
    /// Missing anchor, stale anchor treated as unusable, or TWAP degradation to spot.
    degraded_dual_source: bool,
    /// Final price outside `[min, max]` sanity bounds is tolerated when
    /// the policy also tolerates unsafe deviation.
    sanity_violation: bool,
}

impl Allowances {
    const fn for_policy(p: OraclePolicy) -> Self {
        use OraclePolicy::*;
        match p {
            RiskIncreasing => Allowances {
                disabled_market: false,
                stale_source: false,
                unsafe_deviation: false,
                degraded_dual_source: false,
                sanity_violation: false,
            },
            RiskDecreasing => Allowances {
                disabled_market: false,
                stale_source: true,
                unsafe_deviation: true,
                degraded_dual_source: true,
                sanity_violation: true,
            },
            Repay => Allowances {
                disabled_market: true,
                stale_source: true,
                unsafe_deviation: true,
                degraded_dual_source: true,
                sanity_violation: true,
            },
            // Rejects each loosening like RiskIncreasing so seizure accounting
            // cannot read a degraded price; kept distinct for intent/auditing.
            Liquidation => Allowances {
                disabled_market: false,
                stale_source: false,
                unsafe_deviation: false,
                degraded_dual_source: false,
                sanity_violation: false,
            },
            View => Allowances {
                disabled_market: true,
                stale_source: true,
                unsafe_deviation: true,
                degraded_dual_source: true,
                sanity_violation: true,
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

    pub fn allows_degraded_dual_source(self) -> bool {
        Allowances::for_policy(self).degraded_dual_source
    }

    pub fn allows_sanity_violation(self) -> bool {
        Allowances::for_policy(self).sanity_violation
    }

    pub const fn requires_blended_first_band(self) -> bool {
        matches!(
            self,
            OraclePolicy::RiskIncreasing | OraclePolicy::Liquidation
        )
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
}
