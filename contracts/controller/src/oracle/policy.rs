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
#[path = "../../tests/oracle/policy.rs"]
mod tests;
