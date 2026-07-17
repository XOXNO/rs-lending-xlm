//! Macro attaching Certora summaries inside the controller.

#[doc(hidden)]
#[macro_export]
// Bare `crate::` is valid: macro is only invoked inside the controller crate.
#[allow(clippy::crate_in_macro_def)]
macro_rules! summarized {
    ($($summary:ident)::+, $($body:tt)*) => {
        cvlr_soroban_macros::apply_summary!(crate::spec::summaries::$($summary)::+, $($body)*);
    };
}
