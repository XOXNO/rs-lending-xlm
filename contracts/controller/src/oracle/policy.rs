// Oracle policy per entry point.

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
                unsafe_deviation: true,
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
        assert!(p.allows_unsafe_deviation());
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
}
