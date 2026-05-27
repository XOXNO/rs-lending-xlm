//! Leveraged / rebalancing strategies + flash-loan surface.
//!
//! This module groups all flows that either (a) cross the external
//! aggregator router or (b) require the flash-loan single-flight guard.
//!
//! ## Why these files live together
//!
//! - The four "strategy" entrypoints (`multiply`, `swap_collateral`,
//!   `swap_debt`, `repay_debt_with_collateral`) all accept an
//!   `AggregatorSwap` and must defend against a malicious router using
//!   exactly the same pre-snapshot / post-delta verification (ADR 0005).
//!   The shared code lives in `helpers`.
//! - `flash_loan` (the user-facing flash-loan primitive) uses the same
//!   guard flag and the same pool callback pattern. Placing it here is an
//!   implementation detail, not a statement that flash loans are
//!   "strategies". It keeps the guarded-execution machinery in one place.
//!
//! All strategy flows still go through the normal position primitives
//! (`positions::borrow`, `positions::withdraw`, etc.) and therefore
//! inherit the same risk model, oracle policy, and event batching. The
//! only extra surface is the untrusted router boundary.
//!
//! See `helpers.rs` for the balance-delta machinery and the router client
//! trait definition. The four strategy entrypoints and `helpers` stay
//! together because they share the exact same untrusted-router defense
//! (ADR 0005); splitting would diffuse that invariant. The surface has
//! four callers inside the crate and zero Certora harness references.

mod flash_loan;
pub(crate) mod helpers;
mod multiply;
mod repay_debt_with_collateral;
mod swap_collateral;
mod swap_debt;
