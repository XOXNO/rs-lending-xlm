#[doc(hidden)]
#[macro_export]
macro_rules! summarized {
    ($($summary:ident)::+, $($body:tt)*) => {
        cvlr_soroban_macros::apply_summary!(crate::spec::summaries::$($summary)::+, $($body)*);
    };
}
