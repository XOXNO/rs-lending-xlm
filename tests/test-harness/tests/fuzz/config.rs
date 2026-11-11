use proptest::prelude::ProptestConfig;
use proptest::test_runner::FileFailurePersistence;

fn checked_cases(cases: u32) -> u32 {
    assert!(cases > 0, "PROPTEST_CASES must be greater than zero");
    cases
}

pub fn cases(default: u32) -> u32 {
    checked_cases(
        std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(default),
    )
}

pub fn config(default_cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases: cases(default_cases),

        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
            "proptest-regressions",
        ))),
        ..ProptestConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::checked_cases;

    #[test]
    fn positive_case_counts_are_preserved() {
        assert_eq!(checked_cases(1), 1);
        assert_eq!(checked_cases(256), 256);
    }

    #[test]
    #[should_panic(expected = "PROPTEST_CASES must be greater than zero")]
    fn zero_case_count_is_rejected() {
        checked_cases(0);
    }
}
