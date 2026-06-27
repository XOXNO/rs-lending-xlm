//! Per-entrypoint oracle failure policy.
//! Each controller flow chooses one variant before reading prices.
//! The variant decides which oracle failures the flow may tolerate.
//!
//! `RiskIncreasing` and `Liquidation` tolerate nothing (every `allows_*`
//! returns false) so borrow and seizure accounting never read a degraded
//! price. Any future variant defaults to that fail-closed behaviour until it
//! is explicitly listed in a getter.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OraclePolicy {
    RiskIncreasing,
    RiskDecreasing,
    Repay,
    Liquidation,
    View,
}

impl OraclePolicy {
    pub fn allows_disabled_market(self) -> bool {
        matches!(self, Self::Repay | Self::View)
    }

    pub fn allows_stale_source(self) -> bool {
        matches!(self, Self::RiskDecreasing | Self::Repay | Self::View)
    }

    /// Primary/anchor divergence beyond the last tolerance band.
    pub fn allows_unsafe_deviation(self) -> bool {
        matches!(self, Self::RiskDecreasing | Self::Repay | Self::View)
    }

    /// Missing anchor, stale anchor treated as unusable, or TWAP degradation to spot.
    pub fn allows_degraded_dual_source(self) -> bool {
        matches!(self, Self::RiskDecreasing | Self::Repay | Self::View)
    }

    /// Final price outside `[min, max]` sanity bounds; gated together with
    /// `allows_unsafe_deviation`.
    pub fn allows_sanity_violation(self) -> bool {
        matches!(self, Self::RiskDecreasing | Self::Repay | Self::View)
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
