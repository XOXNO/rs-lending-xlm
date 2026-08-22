pub use test_harness::{xlm_preset, LendingTest};

pub fn build_wide_context() -> LendingTest {
    LendingTest::new()
        .standard_two_asset()
        .with_market(xlm_preset())
        .build()
}
