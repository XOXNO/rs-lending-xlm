#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OraclePolicy {
    RiskIncreasing,
    RiskDecreasing,
    Repay,
    IsolatedRepay,
    View,
}

impl OraclePolicy {
    pub fn allows_disabled_market(self) -> bool {
        matches!(
            self,
            OraclePolicy::Repay | OraclePolicy::IsolatedRepay | OraclePolicy::View
        )
    }

    pub fn allows_stale_source(self) -> bool {
        matches!(
            self,
            OraclePolicy::RiskDecreasing | OraclePolicy::Repay | OraclePolicy::View
        )
    }

    pub fn allows_unsafe_deviation(self) -> bool {
        matches!(
            self,
            OraclePolicy::RiskDecreasing | OraclePolicy::Repay | OraclePolicy::View
        )
    }

    pub fn allows_missing_twap_fallback(self) -> bool {
        matches!(
            self,
            OraclePolicy::RiskDecreasing | OraclePolicy::Repay | OraclePolicy::View
        )
    }
}
