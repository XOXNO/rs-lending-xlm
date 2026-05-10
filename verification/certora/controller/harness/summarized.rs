#[doc(hidden)]
#[macro_export]
// `crate::spec::summaries::...` is intentional: this macro is only invoked from
// within the controller crate, where `spec::summaries` is defined. `$crate`
// would point to the macro-defining crate, which is the same crate here, so
// either form is functionally identical — the bare `crate::` keeps the call
// site readable.
#[allow(clippy::crate_in_macro_def)]
macro_rules! summarized {
    ($($summary:ident)::+, $($body:tt)*) => {
        cvlr_soroban_macros::apply_summary!(crate::spec::summaries::$($summary)::+, $($body)*);
    };
}
