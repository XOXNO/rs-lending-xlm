//! Leveraged / rebalancing strategies + flash-loan surface.
//!
//! Groups all flows that cross the external aggregator router or need the
//! flash-loan single-flight guard.
//!
//! The four strategy entrypoints (`multiply`, `swap_collateral`, `swap_debt`,
//! `repay_debt_with_collateral`) accept an `AggregatorSwap` and share the exact
//! pre-snapshot / post-delta defense against a malicious router (ADR 0005); that
//! shared code (balance-delta machinery + router client trait) lives in `helpers`,
//! kept together so the invariant is not diffused. `flash_loan` sits here too —
//! an implementation detail, not a claim that flash loans are "strategies" —
//! because it uses the same guard flag and pool-callback pattern.
//!
//! All strategy flows still go through the normal position primitives
//! (`positions::borrow`, `positions::withdraw`, …) and inherit the same risk
//! model, oracle policy, and event batching; the only extra surface is the
//! untrusted router boundary.

pub(crate) mod flash_loan;
pub(crate) mod helpers;
mod multiply;
mod repay_debt_with_collateral;
mod swap_collateral;
mod swap_debt;
