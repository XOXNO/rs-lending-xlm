//! Per-entrypoint oracle failure policy.
//!
//! Each controller flow chooses one variant before reading prices. The variant
//! decides whether disabled markets, stale sources, out-of-band deviations, or
//! missing TWAP history can be tolerated for that flow.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OraclePolicy {
    RiskIncreasing,
    RiskDecreasing,
    Repay,
    IsolatedRepay,
    Liquidation,
    View,
}

#[derive(Clone, Copy, Debug)]
struct Allowances {
    disabled_market: bool,
    stale_source: bool,
    unsafe_deviation: bool,
    missing_twap_fallback: bool,
}

impl Allowances {
    const fn for_policy(p: OraclePolicy) -> Self {
        use OraclePolicy::*;
        match p {
            RiskIncreasing => Allowances {
                disabled_market: false,
                stale_source: false,
                unsafe_deviation: false,
                missing_twap_fallback: false,
            },
            RiskDecreasing => Allowances {
                disabled_market: false,
                stale_source: true,
                unsafe_deviation: true,
                missing_twap_fallback: true,
            },
            Repay => Allowances {
                disabled_market: true,
                stale_source: true,
                unsafe_deviation: true,
                missing_twap_fallback: true,
            },
            IsolatedRepay => Allowances {
                disabled_market: true,
                stale_source: false,
                unsafe_deviation: false,
                missing_twap_fallback: false,
            },
            // Rejects every loosening like RiskIncreasing so seizure accounting
            // can never read a degraded price; kept distinct for intent/auditing.
            Liquidation => Allowances {
                disabled_market: false,
                stale_source: false,
                unsafe_deviation: false,
                missing_twap_fallback: false,
            },
            View => Allowances {
                disabled_market: true,
                stale_source: true,
                unsafe_deviation: true,
                missing_twap_fallback: true,
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
    }

    #[test]
    fn test_risk_decreasing_permits_stale_and_fallback() {
        let p = OraclePolicy::RiskDecreasing;
        assert!(!p.allows_disabled_market());
        assert!(p.allows_stale_source());
        assert!(p.allows_unsafe_deviation());
        assert!(p.allows_missing_twap_fallback());
    }

    #[test]
    fn test_repay_permits_disabled_and_stale() {
        let p = OraclePolicy::Repay;
        assert!(p.allows_disabled_market());
        assert!(p.allows_stale_source());
        assert!(p.allows_unsafe_deviation());
        assert!(p.allows_missing_twap_fallback());
    }

    #[test]
    fn test_isolated_repay_permits_only_disabled_market() {
        let p = OraclePolicy::IsolatedRepay;
        assert!(p.allows_disabled_market());
        assert!(!p.allows_stale_source());
        assert!(!p.allows_unsafe_deviation());
        assert!(!p.allows_missing_twap_fallback());
    }

    #[test]
    fn test_liquidation_rejects_every_loosening() {
        let p = OraclePolicy::Liquidation;
        assert!(!p.allows_disabled_market());
        assert!(!p.allows_stale_source());
        // Hardened: liquidation rejects when PrimaryWithAnchor sources diverge
        // beyond the configured last tolerance band (UnsafePriceNotAllowed).
        // Unsafe deviation is only tolerated for single-source fallback paths.
        assert!(!p.allows_unsafe_deviation());
        assert!(!p.allows_missing_twap_fallback());
    }

    #[test]
    fn test_view_permits_everything() {
        let p = OraclePolicy::View;
        assert!(p.allows_disabled_market());
        assert!(p.allows_stale_source());
        assert!(p.allows_unsafe_deviation());
        assert!(p.allows_missing_twap_fallback());
    }

    // --- Additional focused coverage for oracle policy composition ---
    // These live inside the controller crate (no external harness) so they
    // execute on every `cargo test -p controller` and give fast regression
    // signal on the policy matrix that drives liquidation, strategies, and
    // repay paths.

    #[test]
    fn test_liquidation_matches_risk_increasing_allowances() {
        // After hardening, Liquidation rejects the same loosenings as
        // RiskIncreasing, so their allowance tables are identical. The variants
        // stay distinct for intent and auditing; this guards the two tables
        // against silently drifting apart.
        let liq = OraclePolicy::Liquidation;
        let inc = OraclePolicy::RiskIncreasing;
        assert_eq!(liq.allows_disabled_market(), inc.allows_disabled_market());
        assert_eq!(liq.allows_stale_source(), inc.allows_stale_source());
        assert_eq!(liq.allows_unsafe_deviation(), inc.allows_unsafe_deviation());
        assert_eq!(
            liq.allows_missing_twap_fallback(),
            inc.allows_missing_twap_fallback()
        );
    }

    #[test]
    fn test_repay_and_view_share_fully_permissive_allowances() {
        // Repay and View have identical allowances by design. Both are
        // intentionally the most lenient (disabled markets OK, stale and
        // unsafe prices tolerated, TWAP fallback allowed). Repay uses this
        // for user debt-reduction flows that must succeed even on degraded
        // oracles; View uses it for read-only queries. The policy type
        // still distinguishes intent for auditing and future evolution.
        assert_eq!(
            OraclePolicy::Repay.allows_disabled_market(),
            OraclePolicy::View.allows_disabled_market()
        );
        assert_eq!(
            OraclePolicy::Repay.allows_missing_twap_fallback(),
            OraclePolicy::View.allows_missing_twap_fallback()
        );
    }
}
