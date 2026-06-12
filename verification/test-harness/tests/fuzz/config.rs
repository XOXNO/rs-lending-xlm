//! Proptest run sizing: `PROPTEST_CASES=N` overrides per-module defaults.

use proptest::prelude::ProptestConfig;

pub fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

pub fn config(default_cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases: cases(default_cases),
        ..ProptestConfig::default()
    }
}

pub fn config_with_rejects(default_cases: u32, max_global_rejects: u32) -> ProptestConfig {
    ProptestConfig {
        cases: cases(default_cases),
        max_global_rejects,
        ..ProptestConfig::default()
    }
}
