//! Proptest run sizing: `PROPTEST_CASES=N` overrides per-module defaults.

use proptest::prelude::ProptestConfig;
use proptest::test_runner::FileFailurePersistence;

pub fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

pub fn config(default_cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases: cases(default_cases),
        // Keep regressions beside each property module, where they are easy to
        // review and already committed by this repository.
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
            "proptest-regressions",
        ))),
        ..ProptestConfig::default()
    }
}
