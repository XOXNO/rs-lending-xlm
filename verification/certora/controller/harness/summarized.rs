//! Preferred macro for attaching Certora summaries inside the controller.
//!
//! This is the clean, forward-looking mechanism (see the oracle
//! `providers/*/client.rs` + `apply_summary!` pattern). It lets us keep
//! production code readable while still giving the prover sound models for
//! expensive operations.
//!
//! Most existing heavy paths still use the older full module-replacement
//! harnesses (for historical reasons). New or refactored heavy functions
//! should prefer thin wrappers + this macro where possible.

#[doc(hidden)]
#[macro_export]
// `crate::spec::summaries::...` is intentional: this macro is only invoked from
// within the controller crate, where `spec::summaries` is defined. `$crate`
// would point to the macro-defining crate, which is the same crate here, so
// either form is functionally identical — the bare `crate::` keeps the call
// site readable.
//
// This is the preferred attachment point for future apply_summary! wiring
// inside controller (see shared/summaries and the oracle client.rs pattern).
// Currently, most controller summaries are reached via the explicit harness
// module replacements or direct re-exports in harness/*.rs .
#[allow(clippy::crate_in_macro_def)]
macro_rules! summarized {
    ($($summary:ident)::+, $($body:tt)*) => {
        cvlr_soroban_macros::apply_summary!(crate::spec::summaries::$($summary)::+, $($body)*);
    };
}
