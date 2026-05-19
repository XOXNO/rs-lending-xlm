// Oracle policy decides how strict price freshness, deviation, and source
// availability checks are at each entry point. One row per variant in
// `Allowances::for_policy` — adding a policy is one new row, not five
// scattered `matches!` updates.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OraclePolicy {
    // Borrow / withdraw-with-debt / supply-when-isolated: anything that
    // increases protocol exposure. Strict in every dimension.
    RiskIncreasing,
    // Withdraw-without-debt / repay-from-self: reduces exposure. Tolerates
    // stale source, missing TWAP fallback, and unsafe anchor deviation
    // because blocking these paths is itself a risk.
    RiskDecreasing,
    // Third-party repay. Same relaxations as `RiskDecreasing` plus allows
    // a disabled market (so a repaying address can still close a position
    // against a deprecated reserve).
    Repay,
    // Isolated-mode repay. Allows repaying against a disabled market so
    // an isolated borrower can always close their debt even after the
    // collateral's market is deprecated, but keeps strict freshness /
    // deviation / TWAP checks. Trapping isolated borrowers from closing
    // a risk-reducing position would otherwise be possible.
    IsolatedRepay,
    // Liquidation. Tolerates anchor deviation so a liquidation is never
    // hard-blocked when the protocol most needs it, but still requires
    // fresh, dual-sourced prices. On unsafe deviation it resolves to
    // the aggregator (spot) rather than the safe source so liquidation
    // tracks the live market instead of getting stuck behind a slower
    // TWAP. Trade-off: a manipulated spot inside the configured sanity
    // bounds can drive liquidations — mitigated by the sanity-bound
    // circuit breaker, the liquidator's profit motive, and the fact
    // that not liquidating would simply transfer the loss to lenders
    // as bad debt.
    Liquidation,
    // Read-only view path. Maximally permissive so dashboards / SDKs can
    // observe state regardless of feed quality. Never used for state
    // changes.
    View,
}

// What the policy allows. Authoritative table; every predicate below reads
// one field of this struct.
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
            RiskIncreasing => Allowances { disabled_market: false, stale_source: false, unsafe_deviation: false, missing_twap_fallback: false, prefer_aggregator_on_deviation: false },
            RiskDecreasing => Allowances { disabled_market: false, stale_source: true,  unsafe_deviation: true,  missing_twap_fallback: true,  prefer_aggregator_on_deviation: false },
            Repay          => Allowances { disabled_market: true,  stale_source: true,  unsafe_deviation: true,  missing_twap_fallback: true,  prefer_aggregator_on_deviation: false },
            IsolatedRepay  => Allowances { disabled_market: true,  stale_source: false, unsafe_deviation: false, missing_twap_fallback: false, prefer_aggregator_on_deviation: false },
            Liquidation    => Allowances { disabled_market: false, stale_source: false, unsafe_deviation: true,  missing_twap_fallback: false, prefer_aggregator_on_deviation: true  },
            View           => Allowances { disabled_market: true,  stale_source: true,  unsafe_deviation: true,  missing_twap_fallback: true,  prefer_aggregator_on_deviation: false },
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

    // Liquidation must reflect the live market price rather than the
    // slower safe-source on unsafe deviation — pricing collateral at TWAP
    // during a real flash crash would leave the borrower healthy on paper,
    // block liquidation, and grow bad debt for lenders.
    pub fn prefers_aggregator_on_deviation(self) -> bool {
        Allowances::for_policy(self).prefer_aggregator_on_deviation
    }
}
