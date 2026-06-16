//! Macro for attaching Certora summaries inside the controller.
//!
//! Keeps production code readable while giving the prover bounded models for
//! expensive operations.

#[doc(hidden)]
#[macro_export]
// `crate::spec::summaries::...` is intentional: the macro is only invoked from
// within the controller crate, where `spec::summaries` is defined, so the bare
// `crate::` path is equivalent to `$crate` and keeps the call site readable.
#[allow(clippy::crate_in_macro_def)]
macro_rules! summarized {
    ($($summary:ident)::+, $($body:tt)*) => {
        cvlr_soroban_macros::apply_summary!(crate::spec::summaries::$($summary)::+, $($body)*);
    };
}
